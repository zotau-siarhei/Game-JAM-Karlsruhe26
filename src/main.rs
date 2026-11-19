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
mod weapon_sfx;
mod weapons;
mod voice;
mod waves;

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
            std::process::exit(0);
        }
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
    // На Android аргументов запуска нет: развёртку для замера выключает
    // `adb shell setprop debug.spaceguard.novsync 1`.
    #[cfg(target_os = "android")]
    let no_vsync = bench.on || android::system_property(c"debug.spaceguard.novsync").is_some_and(|v| v == "1");
    #[cfg(not(target_os = "android"))]
    let no_vsync = bench.on;
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
            ..default()
        }),
        ..default()
    }).set(AssetPlugin {
        // В браузере каждый ненайденный файл `.meta` — лишний запрос с ошибкой 404,
        // а файлов `.meta` у игры нет.
        meta_check: bevy::asset::AssetMetaCheck::Never,
        ..default()
    });
    // Предобработка мешей только на процессоре, см. `android.rs`. Конвейеры собираются в пуле
    // асинхронных задач, но кадр их ждёт (`prewarm::compile_pipelines`): фоновая сборка вперемешку
    // с отрисовкой вешала GPU Adreno 810 (потеря устройства), а сборка по одному шла 15 с.
    // Свойство `debug.spaceguard.sync 1` возвращает сборку по одному.
    #[cfg(target_os = "android")]
    let synchronous = android::system_property(c"debug.spaceguard.sync").is_some_and(|v| v == "1");
    // Вычислительный пул — системы игры и рендера, запись команд кадра. Bevy отдал бы ему только
    // ядра, оставшиеся от других пулов (из восьми — два), и рендер ждал бы игру; число задано
    // прямо, для замера — свойство `debug.spaceguard.compute`.
    #[cfg(target_os = "android")]
    let compute = android::system_property(c"debug.spaceguard.compute")
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    #[cfg(target_os = "android")]
    let plugins = plugins
        .set(bevy::pbr::PbrPlugin { use_gpu_instance_buffer_builder: false, ..default() })
        .set(bevy::render::RenderPlugin { synchronous_pipeline_compilation: synchronous, ..default() })
        // Половина ядер — на сборку конвейеров при загрузке; в игре этот пул почти не занят.
        .set(bevy::app::TaskPoolPlugin {
            task_pool_options: bevy::app::TaskPoolOptions {
                async_compute: bevy::app::TaskPoolThreadAssignmentPolicy {
                    min_threads: 2,
                    max_threads: 6,
                    percent: 0.5,
                    on_thread_spawn: None,
                    on_thread_destroy: None,
                },
                compute: bevy::app::TaskPoolThreadAssignmentPolicy {
                    min_threads: compute,
                    max_threads: compute,
                    percent: 1.0,
                    on_thread_spawn: None,
                    on_thread_destroy: None,
                },
                ..default()
            },
        });
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
    // winit строит цикл событий один раз на процесс. Android держит процесс после выхода из игры
    // и открывает в нём новую активность (иконка, «открыть» после установки): второй запуск
    // падал с «Failed to build event loop: RecreationAttempt». Процесс уходит вместе с игрой.
    #[cfg(target_os = "android")]
    std::process::exit(0);
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
            hud::HudPlugin,
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
        .add_plugins((
            voice::VoicePlugin,
            hq::HqPlugin,
            sound::SoundPlugin,
            music::MusicPlugin,
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
