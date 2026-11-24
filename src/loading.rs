//! Загрузка при запуске: пока не готово всё, игрок видит только экран загрузки.
//!
//! Игра стоит в `GameState::Loading` под чёрным занавесом (`src/fade.rs`). Мир под ним уже
//! строится и рисуется — так прогреваются шейдеры (`src/prewarm.rs`), — но ничего не звучит и
//! меню не показано. Готово, когда загружены все звуки, реплики и музыка (их ручки списки кладут
//! в [`Preload`]), все модели, закончен прогрев и после него кадры ровные. Тогда надписи гаснут,
//! игра входит в меню, занавес открывается.
//!
//! В браузере свой экран загрузки — страница `web/index.html`: игра под ней держит занавес без
//! надписей, сообщает готовность (`platform::set_warm`) и входит в меню только после нажатия
//! «начать» на странице (`platform::entered`): то же нажатие будит звук.

use bevy::asset::{LoadState, UntypedHandle};
use bevy::prelude::*;

use crate::fade::{ABOVE_CURTAIN_Z, Appear, Curtain};
use crate::models::ModelBank;
use crate::prewarm::Prewarm;
use crate::run::GameState;

/// Экран загрузки виден не меньше, с: без мелькания.
const MIN_SECONDS: f32 = 1.0;
/// Предел загрузки, с: дальше игра входит в меню и пишет, что не догрузилось.
const MAX_SECONDS: f32 = 60.0;
/// Кадров подряд короче [`STEADY_FRAME`] после прогрева — сцена «осела».
const STEADY_FRAMES: u32 = 15;
const STEADY_FRAME: f32 = 0.05;
/// Дольше этого оседания не ждём, с.
const SETTLE_LIMIT: f32 = 3.0;
/// Надписи загрузки гаснут, с; занавес открывается в меню, с.
const TEXT_FADE: f32 = 0.3;
const REVEAL_SECONDS: f32 = 0.8;

/// Доли этапов в полосе прогресса.
const SOUNDS_SHARE: f32 = 0.5;
const MODELS_SHARE: f32 = 0.2;
const SHADERS_SHARE: f32 = 0.25;

const TITLE_COLOR: Color = Color::srgb(1.0, 0.85, 0.3);
const HINT_COLOR: Color = Color::srgb(0.8, 0.8, 0.8);
const BAR_WIDTH: f32 = 520.0;

/// Плагин загрузки.
pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Preload>()
            .init_resource::<LoadingRun>()
            .add_systems(Startup, spawn_screen)
            .add_systems(
                Update,
                (track_loading, draw_screen)
                    .chain()
                    .run_if(in_state(GameState::Loading))
                    .in_set(crate::frame::GameSet::Presentation),
            );
    }
}

/// Ассеты, без которых игра не начинается: звуки списков, реплики, музыка.
#[derive(Resource, Default)]
pub struct Preload {
    handles: Vec<UntypedHandle>,
}

impl Preload {
    /// Учесть ассет в загрузке.
    pub fn track(&mut self, handle: impl Into<UntypedHandle>) {
        self.handles.push(handle.into());
    }

    /// Сколько всего и сколько уже загружено или не загрузилось (ошибка не держит игру).
    fn counts(&self, asset_server: &AssetServer) -> (usize, usize) {
        let done = self
            .handles
            .iter()
            .filter(|handle| {
                matches!(
                    asset_server.load_state(handle.id()),
                    LoadState::Loaded | LoadState::Failed(_) | LoadState::NotLoaded
                )
            })
            .count();
        (self.handles.len(), done)
    }
}

/// Этап загрузки для экрана.
#[derive(Debug, Clone, PartialEq)]
enum Step {
    Sounds { done: usize, total: usize },
    Models { done: usize, total: usize },
    Shaders,
    World,
    Ready,
}

impl Step {
    fn line(&self) -> String {
        match self {
            Step::Sounds { done, total } => format!("LOADING SOUNDS  {done}/{total}"),
            Step::Models { done, total } => format!("LOADING MODELS  {done}/{total}"),
            Step::Shaders => "COMPILING SHADERS".into(),
            Step::World => "PREPARING WORLD".into(),
            Step::Ready => "READY".into(),
        }
    }
}

/// Доля готовности по этапам, от 0 до 1.
fn progress(sounds: (usize, usize), models: (usize, usize), warm: bool, settled: bool) -> f32 {
    let share = |(total, done): (usize, usize)| if total == 0 { 1.0 } else { done as f32 / total as f32 };
    SOUNDS_SHARE * share(sounds)
        + MODELS_SHARE * share(models)
        + SHADERS_SHARE * f32::from(u8::from(warm))
        + (1.0 - SOUNDS_SHARE - MODELS_SHARE - SHADERS_SHARE) * f32::from(u8::from(settled))
}

/// Ход загрузки.
#[derive(Resource, Default)]
struct LoadingRun {
    started: Option<f32>,
    steady: u32,
    settle_waited: f32,
    /// Показанная доля: полоса не идёт назад.
    shown: f32,
    step: Option<Step>,
    /// Когда надписи начали гаснуть.
    leaving: Option<f32>,
}

#[allow(clippy::too_many_arguments)]
fn track_loading(
    time: Res<Time<Real>>,
    asset_server: Res<AssetServer>,
    preload: Res<Preload>,
    bank: Res<ModelBank>,
    prewarm: Res<Prewarm>,
    mut run: ResMut<LoadingRun>,
    mut curtain: ResMut<Curtain>,
    mut next: ResMut<NextState<GameState>>,
    mut screens: Query<(Entity, &mut Appear), With<LoadingScreen>>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let started = *run.started.get_or_insert(now);

    if let Some(leaving) = run.leaving {
        if now - leaving >= TEXT_FADE {
            for (screen, _) in &screens {
                commands.entity(screen).despawn();
            }
            curtain.open(REVEAL_SECONDS);
            next.set(GameState::Menu);
        }
        return;
    }

    let sounds = preload.counts(&asset_server);
    let models_total = bank.requested();
    let models = (models_total, models_total - bank.loading());
    let warm = prewarm.done();
    let loaded = sounds.1 == sounds.0 && models.1 == models.0 && warm;
    if loaded {
        run.steady = if dt < STEADY_FRAME { run.steady + 1 } else { 0 };
        run.settle_waited += dt;
    }
    let settled = loaded && (run.steady >= STEADY_FRAMES || run.settle_waited > SETTLE_LIMIT);
    run.shown = run.shown.max(progress(sounds, models, warm, settled));
    run.step = Some(if sounds.1 < sounds.0 {
        Step::Sounds { done: sounds.1, total: sounds.0 }
    } else if models.1 < models.0 {
        Step::Models { done: models.1, total: models.0 }
    } else if !warm {
        Step::Shaders
    } else if !settled {
        Step::World
    } else {
        Step::Ready
    });
    if let Some(step) = &run.step {
        crate::trail::mark(step.line());
    }

    let elapsed = now - started;
    let timed_out = elapsed > MAX_SECONDS;
    if !(settled && elapsed >= MIN_SECONDS) && !timed_out {
        return;
    }
    // Странице браузера: загрузка закончена, можно предлагать начать.
    crate::platform::set_warm(true);
    if !crate::platform::entered() {
        return;
    }
    if timed_out && !settled {
        warn!(
            "Загрузка не закончилась за {MAX_SECONDS} с: звуки {}/{}, модели {}/{}, шейдеры {}",
            sounds.1, sounds.0, models.1, models.0, if warm { "готовы" } else { "не прогреты" }
        );
    }
    info!("Загрузка: {elapsed:.1} с, звуков {}, моделей {}", sounds.0, models.0);
    run.leaving = Some(now);
    for (_, mut appear) in &mut screens {
        appear.fade_out(TEXT_FADE);
    }
}

/// Экран загрузки поверх занавеса.
#[derive(Component)]
struct LoadingScreen;

#[derive(Component)]
struct StepLine;

#[derive(Component)]
struct BarFill;

/// Экран загрузки: название, этап и полоса. В браузере — только занавес: экран рисует страница.
fn spawn_screen(mut commands: Commands) {
    if cfg!(target_arch = "wasm32") {
        return;
    }
    let line = |text: &str, size: f32, color: Color| {
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
            LoadingScreen,
            Appear::shown(),
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
            GlobalZIndex(ABOVE_CURTAIN_Z),
        ))
        .with_children(|screen| {
            screen.spawn(line("SPACE GUARD", 96.0, TITLE_COLOR));
            screen.spawn(line("SURVIVE AS LONG AS YOU CAN", 26.0, Color::WHITE));
            screen.spawn(line("", 18.0, Color::NONE));
            screen
                .spawn((
                    Node {
                        width: Val::Px(BAR_WIDTH),
                        height: Val::Px(6.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.12)),
                ))
                .with_children(|bar| {
                    bar.spawn((
                        BarFill,
                        Node {
                            width: Val::Percent(0.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(TITLE_COLOR),
                    ));
                });
            screen.spawn((StepLine, line("", 20.0, HINT_COLOR)));
        });
}

fn draw_screen(
    run: Res<LoadingRun>,
    mut fills: Query<&mut Node, With<BarFill>>,
    mut lines: Query<&mut Text, With<StepLine>>,
) {
    for mut node in &mut fills {
        let width = Val::Percent(run.shown * 100.0);
        if node.width != width {
            node.width = width;
        }
    }
    let text = run.step.as_ref().map(Step::line).unwrap_or_default();
    for mut line in &mut lines {
        if line.0 != text {
            line.0.clone_from(&text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn прогресс_складывается_из_этапов() {
        assert_eq!(progress((10, 0), (4, 0), false, false), 0.0);
        let half_sounds = progress((10, 5), (4, 0), false, false);
        assert!((half_sounds - SOUNDS_SHARE / 2.0).abs() < 1e-5);
        assert!((progress((10, 10), (4, 4), true, true) - 1.0).abs() < 1e-5);
        // Пустые списки не держат полосу.
        assert!((progress((0, 0), (0, 0), true, true) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn этап_пишется_латиницей() {
        assert_eq!(Step::Sounds { done: 3, total: 9 }.line(), "LOADING SOUNDS  3/9");
        assert_eq!(Step::Shaders.line(), "COMPILING SHADERS");
    }
}
