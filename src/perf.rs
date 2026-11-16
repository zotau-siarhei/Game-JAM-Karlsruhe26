//! Замер скорости игры: оверлей по F3 и прогон `--bench`. Текст оверлея и лога — латиницей:
//! встроенный шрифт Bevy без кириллицы.
//!
//! Время кадра раскладывается по участкам. Основной поток — игровая логика по наборам
//! `GameSet` и движок до и после них. Рендер-поток — наборы `RenderSystems`: он готовит
//! прошлый кадр, пока основной считает новый. Рендер-поток ждёт видеокарту в `PrepareViews`
//! (там берётся следующий кадр окна), поэтому большое время на этом участке значит, что
//! узкое место — видеокарта. Кадр длиннее обоих потоков — основной ждёт рендер на передаче
//! кадра (`Extract`).
//!
//! `--bench` или `--bench=<секунд>`: забег начинается сам, луноход бессмертен, пауза при
//! потере фокуса выключена, вертикальная синхронизация тоже, сводка пишется в лог раз
//! в 5 с, по истечении срока игра закрывается.
//!
//! Отметки времени — системы между наборами `GameSet`: они сами вне наборов, иначе не встать
//! на границу.

use std::sync::{Arc, Mutex};
use bevy::platform::time::Instant;

use bevy::app::SubApp;
use bevy::prelude::*;
use bevy::render::{Render, RenderApp, RenderSystems};

use crate::enemy::Enemy;
use crate::frame::GameSet;
use crate::quality::Quality;

/// Окно усреднения оверлея, с.
const OVERLAY_WINDOW: f32 = 0.5;
/// Окно усреднения сводки в логе прогона, с.
const LOG_WINDOW: f32 = 5.0;
/// Сколько самых тяжёлых участков потока показывать в оверлее.
const TOP: usize = 4;

/// Наборы основного потока по порядку кадра.
const GAME_SETS: [(GameSet, &str); 10] = [
    (GameSet::Screens, "Screens"),
    (GameSet::Reset, "Reset"),
    (GameSet::Input, "Input"),
    (GameSet::Simulation, "Simulation"),
    (GameSet::Damage, "Damage"),
    (GameSet::ApplyDamage, "ApplyDamage"),
    (GameSet::Death, "Death"),
    (GameSet::Cleanup, "Cleanup"),
    (GameSet::Streaming, "Streaming"),
    (GameSet::Presentation, "Presentation"),
];
/// Участки основного потока: движок до `Update`, наборы игры, движок после.
const MAIN_PARTS: usize = GAME_SETS.len() + 2;

/// Наборы рендер-потока по порядку.
const RENDER_SETS: [(RenderSystems, &str); 10] = [
    (RenderSystems::ExtractCommands, "ExtractCommands"),
    (RenderSystems::PrepareMeshes, "PrepareMeshes"),
    (RenderSystems::CreateViews, "CreateViews"),
    (RenderSystems::Specialize, "Specialize"),
    (RenderSystems::PrepareViews, "PrepareViews (GPU wait)"),
    (RenderSystems::Queue, "Queue"),
    (RenderSystems::PhaseSort, "PhaseSort"),
    (RenderSystems::Prepare, "Prepare"),
    (RenderSystems::Render, "Render (graph, submit)"),
    (RenderSystems::Cleanup, "Cleanup"),
];
const RENDER_PARTS: usize = RENDER_SETS.len();

/// Прогон для замера: параметр запуска `--bench` или `--bench=<секунд>`.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct Bench {
    pub on: bool,
    /// Через сколько секунд закрыть игру; без числа — не закрывать.
    pub seconds: Option<f32>,
}

impl Bench {
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Self {
        let mut bench = Bench::default();
        for arg in args {
            if arg == "--bench" {
                bench.on = true;
            } else if let Some(value) = arg.strip_prefix("--bench=") {
                bench.on = true;
                bench.seconds = value.parse().ok().filter(|s: &f32| *s > 0.0);
            }
        }
        bench
    }
}

/// Накопленные за окно суммы.
#[derive(Default, Clone)]
struct Window {
    frames: u32,
    frame_sum: f32,
    frame_worst: f32,
    main: [f32; MAIN_PARTS],
    main_total: f32,
    render_frames: u32,
    render: [f32; RENDER_PARTS],
    render_total: f32,
}

impl Window {
    /// Средние на кадр, мс.
    fn summary(&self) -> Summary {
        let n = self.frames.max(1) as f32;
        let r = self.render_frames.max(1) as f32;
        Summary {
            fps: if self.frame_sum > 0.0 {
                self.frames as f32 / self.frame_sum
            } else {
                0.0
            },
            frame: self.frame_sum / n * 1000.0,
            worst: self.frame_worst * 1000.0,
            main: self.main.map(|s| s / n * 1000.0),
            main_total: self.main_total / n * 1000.0,
            render: self.render.map(|s| s / r * 1000.0),
            render_total: self.render_total / r * 1000.0,
        }
    }
}

struct Summary {
    fps: f32,
    frame: f32,
    worst: f32,
    main: [f32; MAIN_PARTS],
    main_total: f32,
    render: [f32; RENDER_PARTS],
    render_total: f32,
}

/// Отметки времени текущего кадра и два окна: короткое для оверлея, длинное для лога.
#[derive(Default)]
struct Timings {
    main_marks: [Option<Instant>; MAIN_PARTS + 1],
    render_marks: [Option<Instant>; RENDER_PARTS + 1],
    overlay: Window,
    log: Window,
}

impl Timings {
    fn add(&mut self, f: impl Fn(&mut Window)) {
        f(&mut self.overlay);
        f(&mut self.log);
    }
}

/// Отметки общие для основного и рендер-потока.
#[derive(Resource, Clone, Default)]
struct Shared(Arc<Mutex<Timings>>);

/// Оверлей включён.
#[derive(Resource, Default)]
struct Overlay {
    shown: bool,
    since: f32,
}

/// Время с последней строки в логе прогона, с.
#[derive(Resource, Default)]
struct BenchClock {
    log: f32,
    total: f32,
}

#[derive(Component)]
struct OverlayText;

/// Плагин замера.
pub struct PerfPlugin;

impl Plugin for PerfPlugin {
    fn build(&self, app: &mut App) {
        let shared = Shared::default();
        app.init_resource::<Bench>()
            .init_resource::<Overlay>()
            .init_resource::<BenchClock>()
            .insert_resource(shared.clone())
            .add_systems(First, main_mark::<0>)
            .add_systems(Update, main_mark::<1>.before(GameSet::Screens))
            .add_systems(Last, main_mark::<{ MAIN_PARTS }>)
            .add_systems(
                Update,
                (toggle_overlay, update_overlay, log_bench)
                    .chain()
                    .in_set(GameSet::Presentation),
            );
        add_main_marks(app);
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.insert_resource(shared);
            render.add_systems(
                Render,
                render_mark::<0>.before(RenderSystems::ExtractCommands),
            );
            add_render_marks(render);
        }
    }
}

/// Отметка после каждого набора игры: `I` — номер отметки, после набора `I - 2`.
fn add_main_marks(app: &mut App) {
    macro_rules! mark {
        ($($i:literal),*) => {$(
            let mut config = main_mark::<$i>.after(GAME_SETS[$i - 2].0.clone()).into_configs();
            if let Some((next, _)) = GAME_SETS.get($i - 1) {
                config = config.before(next.clone());
            }
            app.add_systems(Update, config);
        )*};
    }
    mark!(2, 3, 4, 5, 6, 7, 8, 9, 10, 11);
}

/// Отметка после каждого набора рендера: `I` — номер отметки, после набора `I - 1`.
fn add_render_marks(render: &mut SubApp) {
    macro_rules! mark {
        ($($i:literal),*) => {$(
            let mut config = render_mark::<$i>.after(RENDER_SETS[$i - 1].0.clone()).into_configs();
            if let Some((next, _)) = RENDER_SETS.get($i) {
                config = config.before(next.clone());
            }
            render.add_systems(Render, config);
        )*};
    }
    mark!(1, 2, 3, 4, 5, 6, 7, 8, 9, 10);
}

fn main_mark<const I: usize>(shared: Res<Shared>) {
    let now = Instant::now();
    let Ok(mut t) = shared.0.lock() else {
        return;
    };
    if I == 0 {
        // Новый кадр: прошлый, если отмечен целиком, идёт в окна.
        if let (Some(start), Some(end)) = (t.main_marks[0], t.main_marks[MAIN_PARTS]) {
            let frame = (now - start).as_secs_f32();
            let marks = t.main_marks;
            let parts: Vec<f32> = marks
                .windows(2)
                .map(|w| match (w[0], w[1]) {
                    (Some(a), Some(b)) if b >= a => (b - a).as_secs_f32(),
                    _ => 0.0,
                })
                .collect();
            let total = (end - start).as_secs_f32();
            t.add(|w| {
                w.frames += 1;
                w.frame_sum += frame;
                w.frame_worst = w.frame_worst.max(frame);
                for (sum, part) in w.main.iter_mut().zip(&parts) {
                    *sum += part;
                }
                w.main_total += total;
            });
        }
        t.main_marks = Default::default();
    }
    t.main_marks[I] = Some(now);
}

fn render_mark<const I: usize>(shared: Res<Shared>) {
    let now = Instant::now();
    let Ok(mut t) = shared.0.lock() else {
        return;
    };
    if I == 0 {
        t.render_marks = Default::default();
    }
    t.render_marks[I] = Some(now);
    if I == RENDER_PARTS {
        let marks = t.render_marks;
        let Some(start) = marks[0] else {
            return;
        };
        let parts: Vec<f32> = marks
            .windows(2)
            .map(|w| match (w[0], w[1]) {
                (Some(a), Some(b)) if b >= a => (b - a).as_secs_f32(),
                _ => 0.0,
            })
            .collect();
        let total = (now - start).as_secs_f32();
        t.add(|w| {
            w.render_frames += 1;
            for (sum, part) in w.render.iter_mut().zip(&parts) {
                *sum += part;
            }
            w.render_total += total;
        });
    }
}

fn main_label(i: usize) -> &'static str {
    match i {
        0 => "engine before Update",
        i if i <= GAME_SETS.len() => GAME_SETS[i - 1].1,
        _ => "engine after Update",
    }
}

/// Самые тяжёлые участки: «Simulation 2.1, Presentation 1.4, …».
fn top(parts: &[f32], label: impl Fn(usize) -> &'static str, count: usize) -> String {
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by(|a, b| parts[*b].total_cmp(&parts[*a]));
    order
        .into_iter()
        .take(count)
        .map(|i| format!("{} {:.1}", label(i), parts[i]))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Что сейчас в мире: сущности, враги, свет.
fn counts(entities: &bevy::ecs::entity::Entities, enemies: usize, lights: usize) -> String {
    format!(
        "entities {}, enemies {enemies}, point lights {lights}",
        entities.count_spawned()
    )
}

fn toggle_overlay(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut overlay: ResMut<Overlay>,
    texts: Query<Entity, With<OverlayText>>,
) {
    if !keys.just_pressed(KeyCode::F3) {
        return;
    }
    overlay.shown = !overlay.shown;
    for entity in &texts {
        commands.entity(entity).despawn();
    }
    if overlay.shown {
        overlay.since = OVERLAY_WINDOW;
        commands.spawn((
            OverlayText,
            Text::new("measuring..."),
            TextFont {
                font_size: FontSize::Px(14.0),
                ..default()
            },
            TextColor(Color::srgb(0.85, 1.0, 0.85)),
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(10.0),
                bottom: Val::Px(10.0),
                padding: UiRect::all(Val::Px(6.0)),
                ..default()
            },
            GlobalZIndex(1000),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn update_overlay(
    time: Res<Time<Real>>,
    shared: Res<Shared>,
    quality: Res<Quality>,
    entities: &bevy::ecs::entity::Entities,
    enemies: Query<(), With<Enemy>>,
    lights: Query<&ViewVisibility, With<PointLight>>,
    mut overlay: ResMut<Overlay>,
    mut texts: Query<&mut Text, With<OverlayText>>,
) {
    if !overlay.shown {
        return;
    }
    overlay.since += time.delta_secs();
    if overlay.since < OVERLAY_WINDOW {
        return;
    }
    overlay.since = 0.0;
    let Ok(mut t) = shared.0.lock() else {
        return;
    };
    let s = t.overlay.summary();
    t.overlay = Window::default();
    drop(t);
    let lit = lights.iter().filter(|v| v.get()).count();
    let line = format!(
        "FPS {:.0}  frame {:.1} ms (worst {:.1}), quality {:?}\n\
         main thread {:.1} ms: {}\n\
         render thread {:.1} ms: {}\n\
         {}",
        s.fps,
        s.frame,
        s.worst,
        *quality,
        s.main_total,
        top(&s.main, main_label, TOP),
        s.render_total,
        top(&s.render, |i| RENDER_SETS[i].1, TOP),
        counts(entities, enemies.iter().count(), lit),
    );
    for mut text in &mut texts {
        text.0 = line.clone();
    }
}

#[allow(clippy::too_many_arguments)]
fn log_bench(
    bench: Res<Bench>,
    time: Res<Time<Real>>,
    shared: Res<Shared>,
    quality: Res<Quality>,
    entities: &bevy::ecs::entity::Entities,
    enemies: Query<(), With<Enemy>>,
    lights: Query<&ViewVisibility, With<PointLight>>,
    mut clock: ResMut<BenchClock>,
    mut exit: MessageWriter<AppExit>,
) {
    if !bench.on {
        return;
    }
    clock.log += time.delta_secs();
    clock.total += time.delta_secs();
    if clock.log >= LOG_WINDOW {
        clock.log = 0.0;
        let Ok(mut t) = shared.0.lock() else {
            return;
        };
        let s = t.log.summary();
        t.log = Window::default();
        drop(t);
        let lit = lights.iter().filter(|v| v.get()).count();
        info!(
            "Bench {:.0} s: FPS {:.1}, frame {:.2} ms, worst {:.1} ms, quality {:?}; {}",
            clock.total,
            s.fps,
            s.frame,
            s.worst,
            *quality,
            counts(entities, enemies.iter().count(), lit)
        );
        let main: Vec<String> = (0..MAIN_PARTS)
            .map(|i| format!("{} {:.2}", main_label(i), s.main[i]))
            .collect();
        info!("  main thread {:.2} ms: {}", s.main_total, main.join(", "));
        let render: Vec<String> = (0..RENDER_PARTS)
            .map(|i| format!("{} {:.2}", RENDER_SETS[i].1, s.render[i]))
            .collect();
        info!(
            "  render thread {:.2} ms: {}",
            s.render_total,
            render.join(", ")
        );
    }
    if bench.seconds.is_some_and(|limit| clock.total >= limit) {
        info!("Bench finished");
        exit.write(AppExit::Success);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn прогон_включается_параметром_и_берёт_срок() {
        assert_eq!(Bench::from_args(args(&["game"])), Bench::default());
        assert_eq!(
            Bench::from_args(args(&["game", "--bench"])),
            Bench {
                on: true,
                seconds: None
            }
        );
        assert_eq!(
            Bench::from_args(args(&["game", "--bench=90"])),
            Bench {
                on: true,
                seconds: Some(90.0)
            }
        );
        assert_eq!(
            Bench::from_args(args(&["game", "--bench=x"])),
            Bench {
                on: true,
                seconds: None
            }
        );
    }

    #[test]
    fn самые_тяжёлые_участки_идут_первыми() {
        let parts = [0.5, 3.0, 1.0];
        let line = top(&parts, |i| ["a", "b", "c"][i], 2);
        assert_eq!(line, "b 3.0, c 1.0");
    }
}
