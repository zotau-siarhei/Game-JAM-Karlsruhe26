//! Space Guard: луноход попадает на неизвестную планету и отбивается от врагов.
//!
//! Сейчас есть холмистый бесконечный ландшафт, луноход с танковым управлением и камера сверху.

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
mod hq;
mod hud;
mod impact;
mod minimap;
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
mod rockets;
mod rover;
mod rover_look;
mod run;
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
        ))
        .add_systems(Startup, spawn_light)
        .run();
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
