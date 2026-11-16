//! Space Guard: луноход попадает на неизвестную планету и отбивается от врагов.
//!
//! Аркадный шутер с видом сверху: разрушаемый рельеф с кратерами и лавой, посёлки с жителями,
//! железная дорога, волны врагов и босс, шесть видов оружия, звук и голос штаба. Каждая механика —
//! плагин в своём модуле, здесь они только собираются в приложение.

mod aim;
mod airstrike;
mod alarm;
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
mod debug_shot;
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
mod models;
mod laser;
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
mod loading;
mod prewarm;
mod puddles;
mod quality;
mod rails;
mod railway;
mod records;
mod recoil;
mod regen;
mod repair;
mod resident_anim;
mod residents;
mod rng;
mod rockets;
mod rover;
mod rover_look;
mod tank_look;
mod ui_scale;
mod run;
mod service;
mod shock;
mod settlements;
mod sound;
mod subsidence;
mod sound_list;
mod steering;
mod stick_aim;
mod terrain;
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

fn main() {
    // Замер скорости: `cargo run -- --bench=180` (см. `perf.rs`).
    let bench = perf::Bench::from_args(std::env::args());
    // Проверка волн без гибели: `cargo run -- --immortal`; в замере луноход тоже бессмертен.
    let immortal = combat::Immortal(combat::Immortal::from_args(std::env::args()).0 || bench.on);
    App::new()
        .insert_resource(ClearColor(SKY_COLOR))
        .insert_resource(RunSeed::from_clock())
        .insert_resource(immortal)
        .insert_resource(bench)
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Space Guard".into(),
                // В замере кадры не ждут развёртки экрана: видно, сколько игра может.
                present_mode: if bench.on { bevy::window::PresentMode::AutoNoVsync } else { bevy::window::PresentMode::default() },
                // В браузере игра рисует в холст `#space-guard` и занимает всю страницу.
                canvas: cfg!(target_arch = "wasm32").then(|| "#space-guard".into()),
                fit_canvas_to_parent: true,
                ..default()
            }),
            ..default()
        }).set(AssetPlugin {
            // В браузере каждый ненайденный файл `.meta` — лишний запрос с ошибкой 404,
            // а файлов `.meta` у игры нет.
            meta_check: bevy::asset::AssetMetaCheck::Never,
            ..default()
        }))
        // Частицы: плагину нужен рендер, поэтому он стоит здесь, а не среди плагинов игры.
        .add_plugins(bevy_hanabi::HanabiPlugin)
        // Замер ставит отметки и в рендер-потоке, поэтому тоже здесь.
        .add_plugins(perf::PerfPlugin)
        .add_plugins(shock::ShockPlugin)
        .add_plugins((fonts::FontsPlugin, fade::FadePlugin, loading::LoadingPlugin))
        .add_plugins((gamepad::GamepadPlugin, stick_aim::StickAimPlugin, haptics::HapticsPlugin))
        .add_plugins(game_plugins)
        .add_systems(Startup, spawn_light)
        .run();
}

/// Все плагины игры. Отдельной функцией, чтобы тест собирал тот же кадр, что и игра.
fn game_plugins(app: &mut App) {
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
        // Посёлки, объекты поверхности и разрушение.
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
        .add_plugins((fire::FirePlugin, debug_shot::DebugShotPlugin))
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
    ));
}
