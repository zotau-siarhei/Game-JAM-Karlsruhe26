//! Экономия на скелетной анимации жителей и роботов.
//!
//! Скелет Mixamo — около 65 костей, и каждый кадр Bevy пересчитывает позу каждой кости и её
//! `GlobalTransform`. Поэтому анимация вне экрана не считается вовсе, а дальняя — раз в
//! несколько кадров. Вблизи — каждый кадр.
//!
//! Пропуск кадра: граф анимаций (`AnimationGraphHandle`) снимается с проигрывателя и лежит в
//! [`ParkedGraph`]. Без графа Bevy не двигает ни время клипа, ни кости — поза стоит. Когда
//! граф возвращается после короткого пропуска, время клипа догоняет пропущенное, и темп шага
//! не меняется. После долгого ухода с экрана клип продолжается с того же места.
//!
//! Системы, которые вешают граф на новый проигрыватель, пропускают проигрыватели с
//! [`ParkedGraph`]: иначе отложенный граф заменился бы новым.

use bevy::prelude::*;

use crate::camera::{Focus, GameCamera};
use crate::frame::GameSet;

/// Ближе этого к фокусу анимация идёт каждый кадр, м.
const NEAR: f32 = 40.0;
/// Ближе этого — раз в 2 кадра, дальше — раз в 4, м.
const MID: f32 = 70.0;
/// Запас за краем экрана в долях половины экрана: на краю модель видна наполовину.
const SCREEN_MARGIN: f32 = 0.2;
/// Пропуск дольше этого не догоняется: клип просто продолжается, с.
const MAX_CATCH_UP: f32 = 0.25;

/// Граф анимаций, снятый с проигрывателя на время пропуска.
#[derive(Component)]
pub struct ParkedGraph {
    graph: Handle<AnimationGraph>,
    /// Сколько времени пропущено, с.
    skipped: f32,
}

/// Плагин экономии на анимации.
pub struct AnimLodPlugin;

impl Plugin for AnimLodPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, thin_animation.in_set(GameSet::Presentation));
    }
}

/// Раз в сколько кадров считать анимацию: 0 — не считать.
fn step(on_screen: bool, distance: f32) -> u32 {
    if !on_screen {
        0
    } else if distance < NEAR {
        1
    } else if distance < MID {
        2
    } else {
        4
    }
}

/// Считается ли анимация в этом кадре: дальние проигрыватели разнесены по кадрам.
fn due(step: u32, frame: u32, entity: Entity) -> bool {
    step == 1 || (step > 1 && (frame.wrapping_add(entity.to_bits() as u32)).is_multiple_of(step))
}

/// Точка на экране с запасом у края.
fn on_screen(camera: &Camera, camera_transform: &GlobalTransform, point: Vec3) -> bool {
    let Some(ndc) = camera.world_to_ndc(camera_transform, point) else {
        return false;
    };
    let edge = 1.0 + SCREEN_MARGIN;
    ndc.x.abs() <= edge && ndc.y.abs() <= edge && (0.0..=1.0).contains(&ndc.z)
}

#[allow(clippy::type_complexity)]
fn thin_animation(
    mut commands: Commands,
    time: Res<Time>,
    focus: Res<Focus>,
    cameras: Query<(&Camera, &GlobalTransform), With<GameCamera>>,
    playing: Query<
        (Entity, &GlobalTransform, &AnimationGraphHandle),
        (With<AnimationPlayer>, Without<ParkedGraph>),
    >,
    mut parked: Query<
        (
            Entity,
            &GlobalTransform,
            &mut ParkedGraph,
            &mut AnimationPlayer,
        ),
        Without<AnimationGraphHandle>,
    >,
    mut frame: Local<u32>,
) {
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    *frame = frame.wrapping_add(1);
    let wanted = |entity: Entity, point: Vec3| {
        let distance = (point - focus.0).xz().length();
        due(
            step(on_screen(camera, camera_transform, point), distance),
            *frame,
            entity,
        )
    };
    for (entity, transform, graph) in &playing {
        if !wanted(entity, transform.translation()) {
            commands
                .entity(entity)
                .try_remove::<AnimationGraphHandle>()
                .try_insert(ParkedGraph {
                    graph: graph.0.clone(),
                    skipped: 0.0,
                });
        }
    }
    let dt = time.delta_secs();
    for (entity, transform, mut park, mut player) in &mut parked {
        park.skipped += dt;
        if !wanted(entity, transform.translation()) {
            continue;
        }
        // Этот кадр Bevy прибавит сам, догоняются только пропущенные.
        let behind = park.skipped - dt;
        if behind > 0.0 && behind <= MAX_CATCH_UP {
            for (_, animation) in player.playing_animations_mut() {
                if !animation.is_finished() && !animation.is_paused() {
                    let time = animation.seek_time() + behind * animation.speed();
                    animation.set_seek_time(time);
                }
            }
        }
        commands
            .entity(entity)
            .try_remove::<ParkedGraph>()
            .try_insert(AnimationGraphHandle(park.graph.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn вне_экрана_не_считается_вблизи_каждый_кадр_вдали_реже() {
        assert_eq!(step(false, 5.0), 0);
        assert_eq!(step(true, 10.0), 1);
        assert_eq!(step(true, 50.0), 2);
        assert_eq!(step(true, 100.0), 4);
    }

    #[test]
    fn дальний_проигрыватель_считается_раз_в_несколько_кадров() {
        let entity = Entity::from_raw_u32(7).unwrap();
        let count = |step| (0..12).filter(|&f| due(step, f, entity)).count();
        assert_eq!(count(1), 12);
        assert_eq!(count(2), 6);
        assert_eq!(count(4), 3);
        assert_eq!(count(0), 0);
    }
}
