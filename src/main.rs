//! Space Guard: луноход попадает на неизвестную планету и отбивается от врагов.
//!
//! Аркадный шутер с видом сверху: разрушаемый рельеф с кратерами и лавой, посёлки с жителями,
//! железная дорога, волны врагов и босс, шесть видов оружия, звук и голос штаба. Каждая механика —
//! плагин в своём модуле, здесь они только собираются в приложение.

mod aim;
mod airstrike;
mod alarm;
#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;
mod anim_lod;
mod arsenal;
mod audio_events;
mod boss;
mod buildings;
mod birds;
mod bullet;
mod camera;
mod capsule;
mod casings;
mod collapse;
mod combat;
mod comic;
mod debris;
mod debris_forms;
mod debug_checks;
mod debug_shot;
mod difficulty;
mod craters;
mod enemy;
mod enemy_look;
mod interceptor;
mod explosion;
mod explosion_fx;
mod fire;
mod frame;
mod gamepad;
mod haptics;
mod hq;
mod grass;
mod hud;
mod impact;
mod intro;
mod minimap;
mod menu;
mod models;
mod laser;
mod lod;
mod lava;
mod lava_flow;
mod mortar;
mod obstacles;
mod music;
mod pickups;
mod props;
mod perf;
mod platform;
mod fade;
mod fonts;
mod fragments;
mod loading;
mod prewarm;
mod profile;
mod paths;
mod puddles;
mod quality;
mod rails;
mod railway;
mod records;
mod recoil;
mod render_scale;
mod regen;
mod repair;
mod resident_anim;
mod residents;
mod rng;
mod rockets;
mod rover;
mod rover_look;
mod settings;
mod tank_look;
mod ui_scale;
mod run;
mod service;
mod shock;
mod settlements;
mod showcase;
mod sound;
mod subsidence;
mod sound_list;
mod steering;
mod stick_aim;
mod terrain;
mod touch;
mod tracks;
mod trains;
mod tunnel_view;
mod upgrades;
mod vehicle;
mod trail;
mod version_label;
mod weapon_sfx;
mod weapons;
mod voice;
mod waves;
mod web_audio;

use bevy::prelude::*;

use camera::CameraPlugin;
use rover::RoverPlugin;
use terrain::{RunSeed, TerrainPlugin};

/// Цвет неба над планетой: дневное голубое.
const SKY_COLOR: Color = Color::srgb(0.45, 0.70, 0.95);

/// Цвет солнца: тёплый дневной свет.
const SUN_COLOR: Color = Color::srgb(1.0, 0.94, 0.82);

/// Откуда светит солнце: около 50° над горизонтом и сбоку от камеры, чтобы склоны холмов
/// и стенки кратеров различались по яркости.
const SUN_POSITION: Vec3 = Vec3::new(6.0, 8.0, 3.0);

// На Android игра — библиотека (`android/Cargo.toml`), макрос даёт ей точку входа `android_main`.
#[bevy_main]
fn main() {
    // Вторая активность в живом процессе (старая ещё не закрыта): цикл событий winit второй раз
    // не построить. Процесс уходит, система поднимает активность в новом.
    #[cfg(target_os = "android")]
    {
        static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
            warn!("Вторая активность в живом процессе: процесс уходит, система откроет игру заново");
            trail::fail("exit: activity recreated in a live process");
            std::process::exit(0);
        }
        android::install_panic_hook();
    }
    // На Android аргументов запуска и переменных окружения нет: их задают свойства (`android.rs`).
    #[cfg(target_os = "android")]
    let args = android::debug_launch();
    #[cfg(not(target_os = "android"))]
    let args: Vec<String> = std::env::args().collect();
    // Замер скорости: `cargo run -- --bench=180` (см. `perf.rs`).
    let bench = perf::Bench::from_args(args.clone());
    // Проверка волн без гибели: `cargo run -- --immortal`; в замере луноход тоже бессмертен.
    let immortal = combat::Immortal(combat::Immortal::from_args(args).0 || bench.on);
    // Профиль платформы (`profile.rs`) — после `debug_launch`: переопределения могут прийти и
    // переменными окружения, которые он ставит. На iOS класс устройства спрашивается здесь, в
    // главном потоке.
    let profile = profile::get();
    // След запуска (`trail.rs`): что игра делала, если процесс оборвётся.
    trail::begin();
    // Развёртку для замера выключает и профиль: `novsync 1` или `render.vsync false`.
    let no_vsync = bench.on || !profile.render.vsync;
    let plugins = DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Space Guard".into(),
            // В замере кадры не ждут развёртки экрана: видно, сколько игра может.
            present_mode: if no_vsync { bevy::window::PresentMode::AutoNoVsync } else { bevy::window::PresentMode::default() },
            // В браузере игра рисует в холст `#space-guard` и занимает всю страницу.
            canvas: cfg!(target_arch = "wasm32").then(|| "#space-guard".into()),
            fit_canvas_to_parent: true,
            // На iOS winit делает вид UIKit размером с запрошенное окно (1280×720 точек), а не с экран:
            // кадр выходит больше экрана и обрезается. Полный экран без рамок берёт размер экрана.
            mode: if cfg!(target_os = "ios") {
                bevy::window::WindowMode::BorderlessFullscreen(bevy::window::MonitorSelection::Primary)
            } else {
                bevy::window::WindowMode::Windowed
            },
            // iOS: полоска «Домой» скрыта, свайп снизу сначала только показывает её — левый стик у
            // нижнего края не сворачивает игру.
            prefers_home_indicator_hidden: cfg!(target_os = "ios"),
            preferred_screen_edges_deferring_system_gestures: if cfg!(target_os = "ios") {
                bevy::window::ScreenEdge::Bottom
            } else {
                bevy::window::ScreenEdge::None
            },
            ..default()
        }),
        ..default()
    }).set(bevy::log::LogPlugin {
        // winit на iOS пишет две строки предупреждений на каждый кадр («RedrawRequested during the main
        // event loop», «AboutToWait»): файл лога песочницы рос на мегабайт в минуту.
        filter: format!("{}winit::platform_impl::ios::app_state=error", bevy::log::DEFAULT_FILTER),
        // На iOS лог ещё и в файл песочницы: с телефона системный лог не достать (`ios.rs`).
        #[cfg(target_os = "ios")]
        custom_layer: ios::file_log_layer,
        ..default()
    }).set(AssetPlugin {
        // В браузере каждый ненайденный файл `.meta` — лишний запрос с ошибкой 404,
        // а файлов `.meta` у игры нет.
        meta_check: bevy::asset::AssetMetaCheck::Never,
        ..default()
    });
    // Предобработка мешей только на процессоре, см. `android.rs`: обход драйвера Adreno, не профиль.
    #[cfg(target_os = "android")]
    let plugins = plugins.set(bevy::pbr::PbrPlugin { use_gpu_instance_buffer_builder: false, ..default() });
    // Сборка конвейеров по одному (`prewarm.sync_compile`, синоним `sync 1`); на Android кадр и так
    // ждёт начатые конвейеры (`prewarm::compile_pipelines`).
    let plugins = plugins.set(bevy::render::RenderPlugin {
        synchronous_pipeline_compilation: profile.prewarm.sync_compile,
        // Экономная видеопамять (`render.gpu_memory`): на телефоне видеопамять — общая с системой, а
        // по умолчанию wgpu резервирует её блоками по 128–256 МБ.
        render_creation: bevy::render::settings::RenderCreation::Automatic(Box::new(
            bevy::render::settings::WgpuSettings {
                memory_hints: profile.render.gpu_memory.hints(),
                ..default()
            },
        )),
        ..default()
    });
    let plugins = plugins.set(task_pools(&profile.threads));
    App::new()
        .insert_resource(ClearColor(SKY_COLOR))
        .insert_resource(RunSeed::from_clock())
        .insert_resource(immortal)
        .insert_resource(bench)
        .add_plugins(plugins)
        // Частицы: плагину нужен рендер, поэтому он стоит здесь, а не среди плагинов игры.
        .add_plugins(particles)
        // Замер ставит отметки и в рендер-потоке, поэтому тоже здесь.
        .add_plugins(perf::PerfPlugin)
        .add_plugins(shock::ShockPlugin)
        .add_plugins((fonts::FontsPlugin, fade::FadePlugin, loading::LoadingPlugin))
        .add_plugins((gamepad::GamepadPlugin, stick_aim::StickAimPlugin, haptics::HapticsPlugin, touch::TouchPlugin))
        .add_plugins(game_plugins)
        .add_systems(Startup, spawn_light)
        .run();
    trail::clean("exit");
    // winit строит цикл событий один раз на процесс. Android держит процесс после выхода из игры
    // и открывает в нём новую активность (иконка, «открыть» после установки): второй запуск
    // падал с «Failed to build event loop: RecreationAttempt». Процесс уходит вместе с игрой.
    #[cfg(target_os = "android")]
    std::process::exit(0);
}

/// Пулы потоков по профилю. Вычислительный пул — системы игры и рендера, запись команд кадра:
/// на телефоне Bevy отдал бы ему только ядра, оставшиеся от других пулов (из восьми — два), и рендер
/// ждал бы игру, поэтому число задано прямо (`threads.compute`, синоним `compute`). Пул асинхронных
/// задач собирает конвейеры при загрузке, в игре он почти не занят.
fn task_pools(threads: &profile::Threads) -> bevy::app::TaskPoolPlugin {
    let mut options = bevy::app::TaskPoolOptions::default();
    if let Some([min, max]) = threads.async_compute {
        options.async_compute = bevy::app::TaskPoolThreadAssignmentPolicy {
            min_threads: min,
            max_threads: max,
            percent: 0.5,
            on_thread_spawn: None,
            on_thread_destroy: None,
        };
    }
    if let Some(compute) = threads.compute {
        options.compute = bevy::app::TaskPoolThreadAssignmentPolicy {
            min_threads: compute,
            max_threads: compute,
            percent: 1.0,
            on_thread_spawn: None,
            on_thread_destroy: None,
        };
    }
    bevy::app::TaskPoolPlugin { task_pool_options: options }
}

/// Частицы `bevy_hanabi`. В симуляторе iOS у GPU нет непрямой отрисовки (`INDIRECT_EXECUTION`),
/// без неё плагин падает при старте: там частиц нет, эффекты остаются ассетами без рисования.
fn particles(app: &mut App) {
    if cfg!(all(target_os = "ios", target_abi = "sim")) {
        app.init_asset::<bevy_hanabi::EffectAsset>();
    } else {
        app.add_plugins(bevy_hanabi::HanabiPlugin);
    }
}

/// Все плагины игры. Отдельной функцией, чтобы тест собирал тот же кадр, что и игра.
fn game_plugins(app: &mut App) {
    #[cfg(target_os = "android")]
    app.add_plugins(android::AndroidPlugin);
    #[cfg(target_os = "ios")]
    app.add_plugins(ios::IosPlugin);
    // Профиль платформы — первым: из него берут значения настройки, качество и меню.
    app.add_plugins(profile::ProfilePlugin);
    // Настройки игрока и сложность: из них берут начальные значения качество и меню.
    app.add_plugins((settings::SettingsPlugin, difficulty::DifficultyPlugin, menu::MenuPlugin));
    // Порядок кадра и игровой генератор случайных чисел.
    app.add_plugins((
        frame::FramePlugin,
        rng::RngPlugin,
        resident_anim::ResidentAnimPlugin,
        enemy_look::EnemyLookPlugin,
        // Анимация вне экрана не считается, дальняя — реже.
        anim_lod::AnimLodPlugin,
        // Здоровье само растёт, если лунохода давно не ранили.
        regen::RegenPlugin,
        // Отдача башни и вспышка у дула.
        recoil::RecoilPlugin,
    ))
        // Интерфейс мельче на маленьком экране (телефон).
        .add_plugins(ui_scale::UiScalePlugin)
        // Посёлки, объекты поверхности и разрушение; куски моделей для обломков.
        .add_plugins(fragments::FragmentsPlugin)
        // Облёт разрушений за экраном рекордов.
        .add_plugins(showcase::ShowcasePlugin)
        .add_plugins((
            settlements::SettlementsPlugin,
            buildings::BuildingsPlugin,
            props::PropsPlugin,
            obstacles::ObstaclesPlugin,
            debris::DebrisPlugin,
            collapse::CollapsePlugin,
            subsidence::SubsidencePlugin,
            arsenal::ArsenalPlugin,
            rockets::RocketsPlugin,
            laser::LaserPlugin,
            airstrike::AirstrikePlugin,
            pickups::PickupsPlugin,
            rails::RailsPlugin,
            trains::TrainsPlugin,
            tracks::TracksPlugin,
        ))
        .add_plugins((render_scale::RenderScalePlugin, lod::LodPlugin))
        .add_plugins((
            TerrainPlugin,
            RoverPlugin,
            models::ModelsPlugin,
            rover_look::RoverLookPlugin,
            tank_look::TankLookPlugin,
            CameraPlugin,
            quality::QualityPlugin,
            prewarm::PrewarmPlugin,
            aim::AimPlugin,
            explosion::ExplosionPlugin,
            explosion_fx::ExplosionFxPlugin,
            mortar::MortarPlugin,
            craters::CratersPlugin,
            lava::LavaPlugin,
            impact::ImpactPlugin,
        ))
        .add_plugins((
            combat::CombatPlugin,
            (hud::HudPlugin, version_label::VersionLabelPlugin, trail::TrailPlugin),
            minimap::MinimapPlugin,
            bullet::BulletPlugin,
            enemy::EnemyPlugin,
            interceptor::InterceptorPlugin,
            boss::BossPlugin,
            waves::WavesPlugin,
            alarm::AlarmPlugin,
            service::ServicePlugin,
            capsule::CapsulePlugin,
            weapons::WeaponsPlugin,
            run::RunPlugin,
            repair::RepairPlugin,
            residents::ResidentsPlugin,
        ))
        // Облицовка окон в туннель: рядом с путями, но в группе путей нет места.
        .add_plugins(tunnel_view::TunnelViewPlugin)
        .add_plugins(grass::GrassPlugin)
        .add_plugins((fire::FirePlugin, debug_shot::DebugShotPlugin, debug_checks::DebugChecksPlugin))
        // Лужи и вода в кратерах.
        .add_plugins(puddles::PuddlesPlugin)
        .add_plugins(paths::PathsPlugin)
        .add_plugins((
            voice::VoicePlugin,
            hq::HqPlugin,
            sound::SoundPlugin,
            music::MusicPlugin,
            web_audio::WebAudioPlugin,
            audio_events::AudioEventsPlugin,
            comic::ComicPlugin,
            intro::IntroPlugin,
            weapon_sfx::WeaponSfxPlugin,
            upgrades::UpgradesPlugin,
            casings::CasingsPlugin,
            birds::BirdsPlugin,
        ));
}

/// Создаёт солнце. Тени — карту, каскады и дальность — задаёт ступень качества в `quality`.
fn spawn_light(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            color: SUN_COLOR,
            ..default()
        },
        Transform::from_translation(SUN_POSITION).looking_at(Vec3::ZERO, Vec3::Y),
        // Солнце видит и слой силуэтов для тени (`lod::SHADOW_LAYER`), камера — только обычный.
        bevy::camera::visibility::RenderLayers::from_layers(&[0, lod::SHADOW_LAYER]),
    ));
}
