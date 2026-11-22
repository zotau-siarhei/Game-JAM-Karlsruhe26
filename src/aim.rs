//! Прицел: точка на поверхности рельефа под курсором и маркер-кольцо в ней.
//!
//! Оружие берёт цель из ресурса `AimPoint`. Точка ищется шагом по лучу из камеры над картой
//! высот, поэтому на склоне и на дне кратера она лежит там, где поверхность видна под курсором.

use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::camera::GameCamera;
use crate::frame::GameSet;
use crate::rover::Rover;
use crate::run::GameState;
use crate::terrain::Terrain;

/// Предел дальности стрельбы по горизонтали от лунохода, м.
pub const MAX_AIM_RANGE: f32 = 40.0;

/// Шаг по лучу при поиске поверхности, м.
const RAY_STEP: f32 = 0.5;

/// Наибольшее число шагов по лучу: хватает и на дно глубокого кратера.
const RAY_MAX_STEPS: usize = 400;

/// Число шагов бинарного уточнения после того, как луч ушёл под поверхность.
const RAY_REFINE_STEPS: usize = 6;

/// Внешний радиус кольца: примерный радиус кратера мортиры, м.
pub(crate) const MARKER_OUTER_RADIUS: f32 = 5.0;

/// Толщина кольца, м.
const MARKER_WIDTH: f32 = 0.35;

/// Радиус точки в центре кольца, м.
const MARKER_DOT_RADIUS: f32 = 0.3;

/// Число отрезков по окружности кольца.
const MARKER_SEGMENTS: usize = 64;

/// Высота кольца над поверхностью, чтобы оно не мерцало вместе с землёй, м.
pub(crate) const MARKER_LIFT: f32 = 0.12;

/// Цвет маркера в пределах дальности.
const MARKER_COLOR: Color = Color::srgb(1.0, 1.0, 1.0);

/// Цвет маркера на пределе дальности.
const MARKER_LIMIT_COLOR: Color = Color::srgb(1.0, 0.55, 0.1);

/// Цвет маркера, когда прицел стиком поймал врага.
const MARKER_LOCKED_COLOR: Color = Color::srgb(1.0, 0.15, 0.1);

/// Точка прицела на поверхности рельефа.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct AimPoint {
    /// Точка на поверхности, куда ударит оружие.
    pub position: Vec3,
    /// Курсор дальше предела: точка перенесена на окружность предела в сторону курсора.
    pub at_limit: bool,
    /// Прицел стиком поймал врага.
    pub locked: bool,
}

/// Маркер точки прицела.
#[derive(Component)]
struct AimMarker;

/// Материалы маркера: обычный, на пределе дальности и на пойманном враге.
#[derive(Resource)]
struct MarkerMaterials {
    normal: Handle<StandardMaterial>,
    limit: Handle<StandardMaterial>,
    locked: Handle<StandardMaterial>,
}

/// Плагин прицела.
pub struct AimPlugin;

impl Plugin for AimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AimPoint>()
            .add_systems(Startup, (spawn_marker, confine_cursor))
            .add_systems(
                Update,
                (
                    // Перед вводом: всё оружие стреляет в точку под курсором этого кадра. Камера
                    // стоит там, где её показал прошлый кадр, — туда игрок и целился.
                    update_aim_point
                        .run_if(in_state(GameState::Playing))
                        .after(GameSet::Reset)
                        .before(GameSet::Input),
                    update_marker.in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Первая точка луча под поверхностью рельефа, уточнённая бинарным поиском.
///
/// Луч идёт шагом `RAY_STEP`. Если за `RAY_MAX_STEPS` шагов поверхность не найдена,
/// возвращает `None`.
pub fn raycast_terrain(terrain: &Terrain, origin: Vec3, direction: Vec3) -> Option<Vec3> {
    let below = |t: f32| {
        let p = origin + direction * t;
        p.y <= terrain.height_at(p.x, p.z)
    };
    if below(0.0) {
        return Some(origin);
    }
    let hit_step = (1..=RAY_MAX_STEPS).find(|&i| below(i as f32 * RAY_STEP))?;
    let (mut above_t, mut below_t) = ((hit_step - 1) as f32 * RAY_STEP, hit_step as f32 * RAY_STEP);
    for _ in 0..RAY_REFINE_STEPS {
        let mid = (above_t + below_t) * 0.5;
        if below(mid) {
            below_t = mid;
        } else {
            above_t = mid;
        }
    }
    let p = origin + direction * below_t;
    Some(Vec3::new(p.x, terrain.height_at(p.x, p.z), p.z))
}

/// Ограничивает точку прицела дальностью от лунохода по горизонтали и сажает её на рельеф.
///
/// Возвращает точку и признак «на пределе дальности».
#[cfg(test)]
pub fn clamp_to_range(terrain: &Terrain, rover: Vec3, target: Vec3) -> (Vec3, bool) {
    clamp_within(terrain, rover, target, MAX_AIM_RANGE)
}

/// Как [`clamp_to_range`], с дальностью `range`.
pub fn clamp_within(terrain: &Terrain, rover: Vec3, target: Vec3, range: f32) -> (Vec3, bool) {
    let offset = Vec2::new(target.x - rover.x, target.z - rover.z);
    if offset.length() <= range {
        return (target, false);
    }
    let flat = Vec2::new(rover.x, rover.z) + offset.normalize() * range;
    (
        Vec3::new(flat.x, terrain.height_at(flat.x, flat.y), flat.y),
        true,
    )
}

/// Оставляет системный курсор видимым и, где система это умеет, внутри окна.
///
/// macOS не поддерживает удержание курсора в окне без его захвата, там курсор только видимый.
/// На Android и iOS удерживать нечего.
fn confine_cursor(mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>) {
    cursor.visible = true;
    if !cfg!(any(target_os = "macos", target_os = "ios", target_os = "android")) {
        cursor.grab_mode = CursorGrabMode::Confined;
    }
}

/// Пересчитывает точку прицела по курсору. Без курсора над окном точка остаётся прежней.
/// С геймпада прицел ставит правый стик (`stick_aim::aim_with_stick`), курсор не трогает его.
pub fn update_aim_point(
    device: Res<crate::gamepad::InputDevice>,
    mut aim: ResMut<AimPoint>,
    terrain: Res<Terrain>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &Transform), With<GameCamera>>,
    rover: Single<&Transform, With<Rover>>,
    targeting: Res<crate::airstrike::Targeting>,
) {
    if device.stick() {
        return;
    }
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let (camera, camera_transform) = *camera;
    // Трансформ камеры уже сдвинут в этом кадре, а глобальный обновится только после Update:
    // берём локальный, у камеры нет родителя.
    let Ok(ray) = camera.viewport_to_world(&GlobalTransform::from(*camera_transform), cursor)
    else {
        return;
    };
    let Some(hit) = raycast_terrain(&terrain, ray.origin, *ray.direction) else {
        return;
    };
    // В наведении авиаудара прицел дальше обычного.
    let range = if targeting.open { crate::airstrike::TARGET_RANGE } else { MAX_AIM_RANGE };
    let (position, at_limit) = clamp_within(&terrain, rover.translation, hit, range);
    *aim = AimPoint {
        position,
        at_limit,
        locked: false,
    };
}

/// Создаёт маркер: кольцо с точкой в центре, которое каждый кадр ложится на рельеф.
fn spawn_marker(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let unlit = |color: Color| StandardMaterial {
        base_color: color,
        unlit: true,
        ..default()
    };
    let normal = materials.add(unlit(MARKER_COLOR));
    let limit = materials.add(unlit(MARKER_LIMIT_COLOR));
    let locked = materials.add(unlit(MARKER_LOCKED_COLOR));
    commands.spawn((
        AimMarker,
        Mesh3d(meshes.add(marker_mesh(|_| 0.0))),
        MeshMaterial3d(normal.clone()),
        Transform::default(),
        // Вершины меняются каждый кадр, а границы меша нет.
        NoFrustumCulling,
    ));
    commands.insert_resource(MarkerMaterials { normal, limit, locked });
}

/// Двигает маркер в точку прицела, кладёт его вершины на рельеф и красит по дальности.
/// Маркер виден только в забеге и только у мортиры: остальному оружию точка падения не нужна.
/// С геймпадом курсора нет, и маркер виден у любого оружия: он показывает, куда смотрит стик.
/// Вершины пересчитываются, только когда маркер сдвинулся или под ним изменился рельеф.
#[allow(clippy::too_many_arguments)]
fn update_marker(
    state: Res<State<GameState>>,
    device: Res<crate::gamepad::InputDevice>,
    aim: Res<AimPoint>,
    weapon: Res<crate::weapons::CurrentWeapon>,
    terrain: Res<Terrain>,
    materials: Res<MarkerMaterials>,
    mut meshes: ResMut<Assets<Mesh>>,
    marker: Single<
        (
            &mut Transform,
            &Mesh3d,
            &mut MeshMaterial3d<StandardMaterial>,
            &mut Visibility,
        ),
        With<AimMarker>,
    >,
    mut changed: MessageReader<crate::subsidence::TerrainChanged>,
    mut rebuilt: MessageReader<crate::run::WorldRebuilt>,
    mut drawn_at: Local<Option<Vec3>>,
) {
    let (mut transform, mesh, mut material, mut visibility) = marker.into_inner();
    let mortar = *state.get() == GameState::Playing
        && (*weapon == crate::weapons::CurrentWeapon::Mortar || device.stick());
    visibility.set_if_neq(if mortar {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
    let center = aim.position;
    // Меш лежит на рельефе под точкой, где его построили: рельеф там изменился или мир новый —
    // строить заново.
    let dug = changed.read().fold(false, |dug, change| {
        dug || drawn_at.is_some_and(|at| {
            change.center.with_y(0.0).distance(at.with_y(0.0)) < change.radius + MARKER_OUTER_RADIUS
        })
    });
    if dug || rebuilt.read().count() > 0 {
        *drawn_at = None;
    }
    if !mortar {
        return;
    }
    transform.translation = center;
    let wanted = if aim.locked {
        &materials.locked
    } else if aim.at_limit {
        &materials.limit
    } else {
        &materials.normal
    };
    if material.0 != *wanted {
        material.0 = wanted.clone();
    }
    if *drawn_at == Some(center) {
        return;
    }
    if let Some(mut mesh) = meshes.get_mut(&mesh.0) {
        *mesh = marker_mesh(|offset| {
            terrain.height_at(center.x + offset.x, center.z + offset.y) - center.y + MARKER_LIFT
        });
        *drawn_at = Some(center);
    }
}

/// Меш маркера: кольцо и диск в центре. Высота вершины над центром — `lift(смещение по x, z)`.
pub(crate) fn marker_mesh(lift: impl Fn(Vec2) -> f32) -> Mesh {
    let mut positions = Vec::with_capacity(MARKER_SEGMENTS * 3 + 1);
    let mut indices = Vec::with_capacity(MARKER_SEGMENTS * 9);
    let inner = MARKER_OUTER_RADIUS - MARKER_WIDTH;
    let point = |offset: Vec2| [offset.x, lift(offset), offset.y];

    // Кольцо: пары вершин изнутри и снаружи.
    for i in 0..MARKER_SEGMENTS {
        let dir = Vec2::from_angle(i as f32 / MARKER_SEGMENTS as f32 * std::f32::consts::TAU);
        positions.push(point(dir * inner));
        positions.push(point(dir * MARKER_OUTER_RADIUS));
    }
    for i in 0..MARKER_SEGMENTS {
        let (a, b) = (2 * i as u32, 2 * ((i + 1) % MARKER_SEGMENTS) as u32);
        indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
    }

    // Диск в центре веером.
    let center = positions.len() as u32;
    positions.push(point(Vec2::ZERO));
    for i in 0..MARKER_SEGMENTS {
        let dir = Vec2::from_angle(i as f32 / MARKER_SEGMENTS as f32 * std::f32::consts::TAU);
        positions.push(point(dir * MARKER_DOT_RADIUS));
    }
    for i in 0..MARKER_SEGMENTS as u32 {
        let next = (i + 1) % MARKER_SEGMENTS as u32;
        indices.extend_from_slice(&[center, center + 1 + next, center + 1 + i]);
    }

    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(bevy::mesh::Indices::U32(indices))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_from_above_hits_terrain_height() {
        let mut terrain = Terrain::new(23);
        terrain.chunk(IVec2::ZERO);
        for &(x, z) in &[(3.3, 4.7), (17.5, 9.25), (30.0, 30.0)] {
            let ground = terrain.height_at(x, z);
            let origin = Vec3::new(x, ground + 32.0, z + 4.0);
            let direction = (Vec3::new(x, ground, z) - origin).normalize();
            let hit = raycast_terrain(&terrain, origin, direction).expect("луч не нашёл рельеф");
            assert!(
                (hit.y - terrain.height_at(hit.x, hit.z)).abs() < 1e-3,
                "точка не на поверхности: {hit}"
            );
            // Луч смотрит в точку на поверхности, и попадание рядом с ней.
            let miss = Vec2::new(hit.x - x, hit.z - z).length();
            assert!(miss < 0.1, "промах {miss} м в ({x}, {z})");
        }
    }

    #[test]
    fn ray_finds_crater_floor() {
        let mut terrain = Terrain::new(4);
        let chunk = terrain.chunk_mut(IVec2::ZERO);
        // Глубокая яма вокруг вершины (10, 10).
        for iz in 8..=12 {
            for ix in 8..=12 {
                chunk.heights[crate::terrain::ChunkData::index(ix, iz)] -= 12.0;
            }
        }
        let floor = terrain.height_at(10.0, 10.0);
        let origin = Vec3::new(10.0, floor + 40.0, 10.0);
        let hit = raycast_terrain(&terrain, origin, Vec3::NEG_Y).unwrap();
        assert!((hit.y - floor).abs() < 1e-3, "{} вместо дна {floor}", hit.y);
    }

    #[test]
    fn far_target_is_clamped_to_range() {
        let mut terrain = Terrain::new(8);
        terrain.chunk(IVec2::ZERO);
        let rover = Vec3::new(2.0, 0.0, 3.0);
        let (near, near_limit) = clamp_to_range(&terrain, rover, Vec3::new(12.0, 1.0, 3.0));
        assert!(!near_limit);
        assert_eq!(near, Vec3::new(12.0, 1.0, 3.0));

        let (far, far_limit) = clamp_to_range(&terrain, rover, Vec3::new(102.0, 0.0, 3.0));
        assert!(far_limit);
        let distance = Vec2::new(far.x - rover.x, far.z - rover.z).length();
        assert!((distance - MAX_AIM_RANGE).abs() < 1e-3);
        assert!((far.y - terrain.height_at(far.x, far.z)).abs() < 1e-5);
    }
}
