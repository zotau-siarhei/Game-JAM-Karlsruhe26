//! Lunokhod: луноход попадает на неизвестную планету и отбивается от врагов.
//!
//! Сейчас есть холмистый бесконечный ландшафт, луноход с танковым управлением и камера сверху.

mod aim;
mod arsenal;
mod audio_events;
mod buildings;
mod bullet;
mod camera;
mod capsule;
mod collapse;
mod combat;
mod debris;
mod craters;
mod enemy;
mod explosion;
mod explosion_fx;
mod hq;
mod hud;
mod minimap;
mod mortar;
mod obstacles;
mod music;
mod props;
mod records;
mod repair;
mod rover;
mod rover_look;
mod run;
mod settlements;
mod sound;
mod subsidence;
mod sound_list;
mod terrain;
mod vehicle;
mod weapons;
mod voice;
mod waves;

use bevy::prelude::*;

use camera::CameraPlugin;
use rover::RoverPlugin;
use terrain::{RunSeed, TerrainPlugin};

/// Цвет неба над планетой.
const SKY_COLOR: Color = Color::srgb(0.02, 0.02, 0.06);

fn main() {
    App::new()
        .insert_resource(ClearColor(SKY_COLOR))
        .insert_resource(RunSeed::from_clock())
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Lunokhod".into(),
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
        ))
        .add_plugins((
            TerrainPlugin,
            RoverPlugin,
            rover_look::RoverLookPlugin,
            CameraPlugin,
            aim::AimPlugin,
            explosion::ExplosionPlugin,
            explosion_fx::ExplosionFxPlugin,
            mortar::MortarPlugin,
            craters::CratersPlugin,
        ))
        .add_plugins((
            combat::CombatPlugin,
            hud::HudPlugin,
            minimap::MinimapPlugin,
            bullet::BulletPlugin,
            enemy::EnemyPlugin,
            waves::WavesPlugin,
            capsule::CapsulePlugin,
            weapons::WeaponsPlugin,
            run::RunPlugin,
            repair::RepairPlugin,
        ))
        .add_plugins((
            voice::VoicePlugin,
            hq::HqPlugin,
            sound::SoundPlugin,
            music::MusicPlugin,
            audio_events::AudioEventsPlugin,
        ))
        .add_systems(Startup, spawn_light)
        .run();
}

/// Создаёт солнце: невысокое, чтобы склоны холмов различались по яркости.
fn spawn_light(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(6.0, 8.0, 3.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}
