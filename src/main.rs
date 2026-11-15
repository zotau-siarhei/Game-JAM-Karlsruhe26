//! Space Guard: луноход попадает на неизвестную планету и отбивается от врагов.
//!
//! Аркадный шутер с видом сверху: разрушаемый рельеф с кратерами и лавой, посёлки с жителями,
//! железная дорога, волны врагов и босс, шесть видов оружия, звук и голос штаба. Каждая механика —
//! плагин в своём модуле, здесь они только собираются в приложение.

mod aim;
mod airstrike;
mod alarm;
mod arsenal;
mod audio_events;
mod boss;
mod buildings;
mod bullet;
mod camera;
mod capsule;
mod collapse;
mod combat;
mod comic;
mod debris;
mod craters;
mod enemy;
mod explosion;
mod explosion_fx;
mod frame;
mod hq;
mod hud;
mod impact;
mod minimap;
mod models;
mod laser;
mod lava;
mod mortar;
mod obstacles;
mod music;
mod pickups;
mod props;
mod quality;
mod rails;
mod railway;
mod records;
mod repair;
mod residents;
mod rng;
mod rockets;
mod rover;
mod rover_look;
mod run;
mod service;
mod settlements;
mod sound;
mod subsidence;
mod sound_list;
mod steering;
mod terrain;
mod trains;
mod vehicle;
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
    App::new()
        .insert_resource(ClearColor(SKY_COLOR))
        .insert_resource(RunSeed::from_clock())
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Space Guard".into(),
                ..default()
            }),
            ..default()
        }))
        // Частицы: плагину нужен рендер, поэтому он стоит здесь, а не среди плагинов игры.
        .add_plugins(bevy_hanabi::HanabiPlugin)
        .add_plugins(game_plugins)
        .add_systems(Startup, spawn_light)
        .run();
}

/// Все плагины игры. Отдельной функцией, чтобы тест собирал тот же кадр, что и игра.
fn game_plugins(app: &mut App) {
    // Порядок кадра и игровой генератор случайных чисел.
    app.add_plugins((frame::FramePlugin, rng::RngPlugin))
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
        ))
        .add_plugins((
            TerrainPlugin,
            RoverPlugin,
            models::ModelsPlugin,
            rover_look::RoverLookPlugin,
            CameraPlugin,
            quality::QualityPlugin,
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
        .add_plugins((
            voice::VoicePlugin,
            hq::HqPlugin,
            sound::SoundPlugin,
            music::MusicPlugin,
            audio_events::AudioEventsPlugin,
            comic::ComicPlugin,
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
