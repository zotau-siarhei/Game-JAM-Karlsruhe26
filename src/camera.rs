//! Игровая камера: смотрит на луноход сверху с небольшим наклоном и плавно следует за ним.

use bevy::prelude::*;

use crate::rover::{self, Rover};

/// Положение камеры относительно лунохода: над ним и южнее, взгляд под углом около 45° к земле, м.
const CAMERA_OFFSET: Vec3 = Vec3::new(0.0, 25.0, 25.0);

/// Скорость, с которой камера догоняет луноход, 1/с.
const FOLLOW_DECAY: f32 = 8.0;

/// Игровая камера.
#[derive(Component)]
pub struct GameCamera;

/// Плагин камеры.
pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, spawn_camera)
            .add_systems(Update, follow_rover.after(rover::drive_rover));
    }
}

/// Ставит камеру над луноходом. Поворот задаётся один раз и больше не меняется.
fn spawn_camera(mut commands: Commands, rover: Single<&Transform, With<Rover>>) {
    let target = rover.translation;
    commands.spawn((
        GameCamera,
        Camera3d::default(),
        Transform::from_translation(target + CAMERA_OFFSET).looking_at(target, Vec3::NEG_Z),
    ));
}

/// Плавно двигает камеру за луноходом, не вращая её.
pub fn follow_rover(
    time: Res<Time>,
    rover: Single<&Transform, (With<Rover>, Without<GameCamera>)>,
    mut camera: Single<&mut Transform, With<GameCamera>>,
) {
    let target = rover.translation + CAMERA_OFFSET;
    camera
        .translation
        .smooth_nudge(&target, FOLLOW_DECAY, time.delta_secs());
}
