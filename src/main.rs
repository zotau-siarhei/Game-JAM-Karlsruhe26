//! Lunokhod: луноход попадает на неизвестную планету и отбивается от врагов.
//!
//! Сейчас есть холмистый бесконечный ландшафт, луноход с танковым управлением и камера сверху.

mod aim;
mod audio_events;
mod camera;
mod combat;
mod hq;
mod hud;
mod music;
mod records;
mod rover;
mod sound;
mod sound_list;
mod terrain;
mod voice;

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
        .add_plugins((TerrainPlugin, RoverPlugin, CameraPlugin, aim::AimPlugin))
        .add_plugins((combat::CombatPlugin, hud::HudPlugin))
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
