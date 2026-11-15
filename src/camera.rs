//! Игровая камера: смотрит на фокус сверху с небольшим наклоном и плавно следует за ним.
//!
//! Фокус — точка, вокруг которой показан и подгружен мир. В забеге это луноход, в меню —
//! точка, летящая над планетой (её двигает `run`). Камера и подгрузка рельефа читают только
//! [`Focus`], а не луноход.

use bevy::prelude::*;

use crate::frame::GameSet;
use crate::rover::Rover;
use crate::run::GameState;

/// Положение камеры относительно фокуса: над ним и южнее, взгляд под углом около 45° к земле, м.
const CAMERA_OFFSET: Vec3 = Vec3::new(0.0, 25.0, 25.0);

/// Скорость, с которой камера догоняет фокус, 1/с.
const FOLLOW_DECAY: f32 = 8.0;

/// Игровая камера.
#[derive(Component)]
pub struct GameCamera;

/// Точка мира, за которой идут камера и подгрузка мира.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct Focus(pub Vec3);

/// Плагин камеры.
pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Focus>()
            .add_systems(PostStartup, (focus_on_rover, spawn_camera).chain())
            .add_systems(
                Update,
                (
                    // До подгрузки мира: участки появляются вокруг сегодняшнего места лунохода.
                    // В меню фокус двигает `run`, а луноход спрятан и стоит на старте.
                    focus_on_rover
                        .run_if(not(in_state(GameState::Menu)))
                        .after(GameSet::Cleanup)
                        .before(GameSet::Streaming),
                    follow_focus.in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Вне меню фокус стоит на луноходе.
fn focus_on_rover(mut focus: ResMut<Focus>, rover: Single<&Transform, With<Rover>>) {
    if focus.0 != rover.translation {
        focus.0 = rover.translation;
    }
}

/// Ставит камеру над фокусом. Поворот задаётся один раз и больше не меняется.
pub fn spawn_camera(mut commands: Commands, focus: Res<Focus>) {
    let target = focus.0;
    commands.spawn((
        GameCamera,
        Camera3d::default(),
        // Без сглаживания MSAA и с простой фильтрацией теней: так плавнее на слабой видеокарте.
        bevy::render::view::Msaa::Off,
        bevy::light::ShadowFilteringMethod::Hardware2x2,
        Transform::from_translation(target + CAMERA_OFFSET).looking_at(target, Vec3::NEG_Z),
    ));
}

/// Плавно двигает камеру за фокусом, не вращая её.
pub fn follow_focus(time: Res<Time>, focus: Res<Focus>, mut camera: Single<&mut Transform, With<GameCamera>>) {
    let target = focus.0 + CAMERA_OFFSET;
    camera
        .translation
        .smooth_nudge(&target, FOLLOW_DECAY, time.delta_secs());
}
