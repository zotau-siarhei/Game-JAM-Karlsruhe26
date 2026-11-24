//! Игровая камера: смотрит на фокус сверху под углом 45° к земле и плавно следует за ним.
//!
//! Фокус — точка, вокруг которой показан и подгружен мир. В забеге это луноход, в меню —
//! точка, летящая над планетой (её двигает `run`). Камера и подгрузка рельефа читают только
//! [`Focus`], а не луноход.
//!
//! Камера не вращается вокруг вертикали: верх экрана всегда север. Когда рельеф между камерой
//! и фокусом закрывает обзор (стенка глубокого кратера, гребень холма), камера плавно
//! поднимается и смотрит круче, до почти отвесного вида, а на открытом месте возвращается
//! к прежнему углу. Ниже рельефа камера не опускается. Угол не бывает положе 45°, поэтому небо
//! в кадр не попадает.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::frame::GameSet;
use crate::rover::Rover;
use crate::run::GameState;
use crate::terrain::Terrain;

/// Расстояние от фокуса до камеры, м: как у прежнего смещения (0, 25, 25).
const CAMERA_DISTANCE: f32 = 35.355;

/// Наклон взгляда к земле в обычном виде и в самом крутом, рад.
const PITCH_DEFAULT: f32 = std::f32::consts::FRAC_PI_4;
const PITCH_STEEPEST: f32 = 80.0 * std::f32::consts::PI / 180.0;

/// Ступени крутизны, которые проверяются на обзор: 0 — обычный вид, 1 — самый крутой.
const STEEPNESS_STEPS: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

/// Высота точки на луноходе, которую камера должна видеть, над фокусом, м.
const LOOK_HEIGHT: f32 = 0.8;

/// Обзор закрыт, если рельеф выше луча хотя бы на столько, м.
const BLOCK_MARGIN: f32 = 0.3;

/// Вернуться к виду положе можно, только если луч до него идёт выше рельефа на столько, м:
/// запас не даёт камере дёргаться на границе.
const RETURN_MARGIN: f32 = 2.0;

/// Сколько точек луча проверяется.
const RAY_SAMPLES: usize = 16;

/// Скорость подъёма и возврата крутизны, долей в секунду.
const STEEPEN_SPEED: f32 = 2.5;
const RELAX_SPEED: f32 = 0.6;

/// Наименьшая высота камеры над рельефом под ней, м.
const GROUND_CLEARANCE: f32 = 3.0;

/// Скорость, с которой камера догоняет фокус, 1/с.
const FOLLOW_DECAY: f32 = 8.0;

/// Обычный угол обзора камеры по вертикали, рад (как у камеры Bevy по умолчанию).
const FOV_DEFAULT: f32 = std::f32::consts::FRAC_PI_4;
/// На узком экране угол обзора растёт не больше этого, рад: шире — сильные искажения по краям.
const FOV_WIDEST: f32 = 55.0 * std::f32::consts::PI / 180.0;
/// Узким считается экран уже этой доли ширины к высоте. Ширина обзора у лунохода на нём
/// держится как на таком экране: половина обзора экрана 16:9.
const NARROW_ASPECT: f32 = 16.0 / 9.0 * 0.5;
/// Дальше этого камера на узком экране не отъезжает: верх кадра остаётся в подгруженном мире
/// (`terrain::LOAD_RADIUS` участков вокруг фокуса).
const ZOOM_FARTHEST: f32 = 1.6;

/// Игровая камера и её текущая крутизна: 0 — обычный вид, 1 — самый крутой.
#[derive(Component)]
pub struct GameCamera {
    steepness: f32,
    /// Во сколько раз камера дальше обычного: на узком экране (телефон вертикально) она
    /// отъезжает, чтобы по бокам было видно, кто атакует.
    zoom: f32,
}

impl Default for GameCamera {
    fn default() -> Self {
        Self { steepness: 0.0, zoom: 1.0 }
    }
}

/// Угол обзора по вертикали и отъезд камеры для экрана с долей ширины к высоте `aspect`.
/// На экране не уже [`NARROW_ASPECT`] — обычные; на узком сначала шире угол, потом дальше камера.
fn view_for_aspect(aspect: f32) -> (f32, f32) {
    if !aspect.is_finite() || aspect <= 0.0 || aspect >= NARROW_ASPECT {
        return (FOV_DEFAULT, 1.0);
    }
    // Во сколько раз надо расширить обзор по горизонтали.
    let widen = NARROW_ASPECT / aspect;
    let half = (FOV_DEFAULT * 0.5).tan();
    let fov = (2.0 * (half * widen).atan()).min(FOV_WIDEST);
    let zoom = (widen * half / (fov * 0.5).tan()).clamp(1.0, ZOOM_FARTHEST);
    (fov, zoom)
}

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
                    // В меню фокус двигает `run`, а луноход спрятан и стоит на старте; после
                    // гибели фокус облетает разрушения (`showcase`).
                    focus_on_rover
                        .run_if(not(in_state(GameState::Menu)))
                        .run_if(not(in_state(GameState::GameOver)))
                        .after(GameSet::Cleanup)
                        .before(GameSet::Streaming),
                    (fit_view, follow_focus).chain().in_set(GameSet::Presentation),
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

/// Смещение камеры от фокуса при крутизне `steepness`: над ним и южнее.
fn camera_offset(steepness: f32) -> Vec3 {
    let pitch = PITCH_DEFAULT + (PITCH_STEEPEST - PITCH_DEFAULT) * steepness.clamp(0.0, 1.0);
    Vec3::new(0.0, pitch.sin(), pitch.cos()) * CAMERA_DISTANCE
}

/// Поворот камеры при крутизне `steepness`: взгляд на фокус, север вверху экрана.
fn camera_rotation(steepness: f32) -> Quat {
    Transform::from_translation(camera_offset(steepness))
        .looking_at(Vec3::ZERO, Vec3::NEG_Z)
        .rotation
}

/// Закрывает ли рельеф `height` луч от `from` до `to` с запасом `margin`. Точки у самого
/// `from` не проверяются: там земля под луноходом.
fn ray_blocked(height: impl Fn(f32, f32) -> f32, from: Vec3, to: Vec3, margin: f32) -> bool {
    (1..=RAY_SAMPLES).any(|i| {
        let point = from.lerp(to, i as f32 / RAY_SAMPLES as f32);
        height(point.x, point.z) + margin > point.y
    })
}

/// Наименьшая ступень крутизны, с которой точка `look` видна. Ступени положе нынешней
/// `current` требуют большего запаса. Если обзор закрыт отовсюду — самая крутая.
fn needed_steepness(height: impl Fn(f32, f32) -> f32, look: Vec3, current: f32, zoom: f32) -> f32 {
    STEEPNESS_STEPS
        .iter()
        .copied()
        .find(|&steepness| {
            let margin = if steepness < current { RETURN_MARGIN } else { BLOCK_MARGIN };
            !ray_blocked(&height, look, look + camera_offset(steepness) * zoom, margin)
        })
        .unwrap_or(1.0)
}

/// Ставит камеру над фокусом в обычном виде.
pub fn spawn_camera(mut commands: Commands, focus: Res<Focus>) {
    commands.spawn((
        GameCamera::default(),
        Camera3d::default(),
        // Без сглаживания MSAA: так плавнее на слабой видеокарте. Края теней размывает фильтр
        // Gaussian (9 выборок, как ядро 5×5): с одной выборкой Hardware2x2 они шли «пилой». На слабом
        // Android — Hardware2x2: шейдер проще для старых драйверов (`render.shadow_filter` профиля).
        bevy::render::view::Msaa::Off,
        crate::profile::get().render.shadow_filter.method(),
        Transform::from_translation(focus.0 + camera_offset(0.0)).with_rotation(camera_rotation(0.0)),
    ));
}

/// Под форму окна: на узком экране шире угол обзора и камера дальше, на обычном — как было.
pub fn fit_view(
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&mut Projection, &mut GameCamera)>,
) {
    let size = window.resolution.size();
    let (fov, zoom) = view_for_aspect(size.x / size.y);
    let (mut projection, mut camera) = camera.into_inner();
    if (camera.zoom - zoom).abs() > 1e-3 {
        camera.zoom = zoom;
    }
    if let Projection::Perspective(perspective) = projection.as_ref()
        && (perspective.fov - fov).abs() > 1e-3
        && let Projection::Perspective(perspective) = projection.as_mut()
    {
        perspective.fov = fov;
    }
}

/// Вид наведения авиаудара: наклон, рад, и расстояние до фокуса, м. Видимая область — около
/// ±50 м по вертикали экрана и ±85 м по горизонтали: внутри подгруженных ±112 м мира.
const TARGETING_PITCH: f32 = 82.0 * std::f32::consts::PI / 180.0;
const TARGETING_DISTANCE: f32 = CAMERA_DISTANCE * 3.2;

/// Плавно двигает камеру за фокусом. Когда рельеф закрывает фокус, камера поднимается
/// и смотрит круче, потом возвращается; ниже рельефа не опускается.
pub fn follow_focus(
    time: Res<Time>,
    real: Res<Time<Real>>,
    focus: Res<Focus>,
    terrain: Res<Terrain>,
    targeting: Res<crate::airstrike::Targeting>,
    camera: Single<(&mut Transform, &mut GameCamera)>,
    mut was_targeting: Local<bool>,
) {
    let (mut transform, mut camera) = camera.into_inner();
    // В наведении авиаудара время замедлено, а камера идёт по реальному.
    let dt = if targeting.active() { real.delta_secs() } else { time.delta_secs() };
    let height = |x: f32, z: f32| terrain.height_at(x, z);

    let wanted = needed_steepness(height, focus.0 + Vec3::Y * LOOK_HEIGHT, camera.steepness, camera.zoom);
    let step = if wanted > camera.steepness { STEEPEN_SPEED } else { RELAX_SPEED } * dt;
    let steepness = camera.steepness + (wanted - camera.steepness).clamp(-step, step);
    let changed = steepness != camera.steepness;
    camera.steepness = steepness;
    // Переход к виду наведения: плавно выше, круче и дальше.
    let k = targeting.blend * targeting.blend * (3.0 - 2.0 * targeting.blend);
    let top = Vec3::new(0.0, TARGETING_PITCH.sin(), TARGETING_PITCH.cos()) * TARGETING_DISTANCE;
    if changed || targeting.active() || *was_targeting {
        let top_rotation = Transform::from_translation(top).looking_at(Vec3::ZERO, Vec3::NEG_Z).rotation;
        transform.rotation = camera_rotation(steepness).slerp(top_rotation, k);
    }
    *was_targeting = targeting.active();

    let offset = (camera_offset(camera.steepness) * camera.zoom).lerp(top, k);
    let target = focus.0 + offset;
    transform
        .translation
        .smooth_nudge(&target, FOLLOW_DECAY, dt);
    // Под рельеф камера не уходит даже на мгновение, пока догоняет фокус.
    let here = transform.translation;
    let floor = terrain.height_at(here.x, here.z) + GROUND_CLEARANCE;
    if here.y < floor {
        transform.translation.y = floor;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_view_is_the_old_45_degree_offset() {
        let offset = camera_offset(0.0);
        assert!((offset - Vec3::new(0.0, 25.0, 25.0)).length() < 0.01);
        // Самый крутой вид — почти над фокусом, на том же расстоянии.
        let steep = camera_offset(1.0);
        assert!(steep.y > 34.0 && steep.z > 0.0 && steep.z < 7.0);
    }

    #[test]
    fn view_never_reaches_the_horizon() {
        // Взгляд направлен вниз не положе 45°: верх кадра при обзоре 45° не видит неба.
        for steepness in STEEPNESS_STEPS {
            let forward = camera_rotation(steepness) * Vec3::NEG_Z;
            assert!(forward.y <= -std::f32::consts::FRAC_1_SQRT_2 + 1e-4);
        }
    }

    #[test]
    fn wide_screen_keeps_the_usual_view() {
        assert_eq!(view_for_aspect(16.0 / 9.0), (FOV_DEFAULT, 1.0));
        assert_eq!(view_for_aspect(1020.0 / 461.0), (FOV_DEFAULT, 1.0));
    }

    #[test]
    fn portrait_phone_sees_twice_as_wide() {
        let aspect = 461.0 / 1020.0;
        let (fov, zoom) = view_for_aspect(aspect);
        assert!(fov <= FOV_WIDEST + 1e-6 && zoom > 1.0 && zoom <= ZOOM_FARTHEST);
        // Половина ширины обзора у фокуса — как у экрана NARROW_ASPECT, вдвое шире прежней.
        let before = (FOV_DEFAULT * 0.5).tan() * aspect;
        let after = zoom * (fov * 0.5).tan() * aspect;
        assert!(after / before > 1.9, "{}", after / before);
        // Верх кадра смотрит вниз: неба нет, и он не дальше подгруженного мира.
        let top = PITCH_DEFAULT - fov * 0.5;
        assert!(top > 0.2);
        let height = CAMERA_DISTANCE * zoom * PITCH_DEFAULT.sin();
        let reach = height / top.tan() - CAMERA_DISTANCE * zoom * PITCH_DEFAULT.cos();
        assert!(reach < (crate::terrain::LOAD_RADIUS * crate::terrain::CHUNK_SIZE) as f32, "{reach}");
    }

    #[test]
    fn flat_ground_keeps_the_default_view() {
        let flat = |_: f32, _: f32| 0.0;
        assert_eq!(needed_steepness(flat, Vec3::Y, 0.0, 1.0), 0.0);
        assert_eq!(needed_steepness(flat, Vec3::Y, 1.0, 1.0), 0.0);
    }

    #[test]
    fn crater_wall_to_the_south_makes_the_view_steeper() {
        // Луноход на дне кратера глубиной 20 м и радиусом 15 м: южная стенка закрывает вид под 45°.
        let crater = |x: f32, z: f32| if x.hypot(z) < 15.0 { -20.0 } else { 0.0 };
        let look = Vec3::new(0.0, -20.0 + LOOK_HEIGHT, 0.0);
        let steepness = needed_steepness(crater, look, 0.0, 1.0);
        assert!(steepness > 0.0);
        assert!(!ray_blocked(crater, look, look + camera_offset(steepness), BLOCK_MARGIN));
    }
}
