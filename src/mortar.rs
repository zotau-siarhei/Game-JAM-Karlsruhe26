//! Мортира (клавиша 2): выстрел по левой кнопке мыши в точку прицела, снаряд по высокой параболе
//! и взрыв при первом касании поверхности.
//!
//! Время полёта одинаковое на любую дальность, поэтому начальная скорость считается из старта,
//! цели и этого времени. Снаряд летит по формуле, а не интегрированием, и не зависит
//! от частоты кадров. Будущая точка падения отмечается красным кольцом на земле.

use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;

use crate::aim::{self, AimPoint};
use crate::combat::{Health, Team};
use crate::explosion::Explosion;
use crate::rover::Rover;
use crate::terrain::Terrain;
use crate::weapons::CurrentWeapon;

/// Время полёта снаряда до точки прицела, с.
const FLIGHT_TIME: f32 = 1.2;

/// Ускорение свободного падения для снаряда, м/с². Высота дуги над хордой — g·T²/8, около 11 м:
/// снаряд перелетает обычные холмы.
const GRAVITY: f32 = 60.0;

/// Перезарядка мортиры, с.
const RELOAD_TIME: f32 = 1.0;

/// Первые доли секунды снаряд не взрывается: на крутом склоне ствол бывает у самой земли, с.
const ARM_TIME: f32 = 0.1;

/// Наибольшее время полёта: если цель ушла вниз, снаряд летит дальше, но не бесконечно, с.
const MAX_FLIGHT_TIME: f32 = 4.0;

/// Шаг проверки касания вдоль траектории, м: тоньше гребня холма.
const HIT_STEP: f32 = 0.25;

/// Число шагов бинарного уточнения точки касания.
const HIT_REFINE_STEPS: usize = 8;

/// Радиус взрыва снаряда мортиры, м.
const EXPLOSION_RADIUS: f32 = 5.0;

/// Мощность взрыва снаряда мортиры: единица, от которой считается всё остальное оружие.
const EXPLOSION_POWER: f32 = 1.0;

/// Ствол над центром корпуса в его системе координат, м.
const MUZZLE_OFFSET: Vec3 = Vec3::new(0.0, 0.7, -0.4);

/// Радиус снаряда, м.
const SHELL_RADIUS: f32 = 0.3;

/// Длительность вспышки у ствола, с.
const FLASH_TIME: f32 = 0.12;

/// Яркость вспышки у ствола в начале, лм.
const FLASH_INTENSITY: f32 = 2_000_000.0;

/// Радиус шара вспышки у ствола в начале, м.
const FLASH_RADIUS: f32 = 0.6;

/// Цвет вспышки у ствола.
const FLASH_COLOR: Color = Color::srgb(1.0, 0.75, 0.35);

/// Отметка падения чуть выше прицела, чтобы не мерцать с ним в одной точке, м.
const IMPACT_MARKER_EXTRA_LIFT: f32 = 0.03;

/// Цвет отметки точки падения.
const IMPACT_MARKER_COLOR: Color = Color::srgb(1.0, 0.15, 0.1);

/// Траектория снаряда: старт и начальная скорость при постоянной гравитации.
#[derive(Clone, Copy, Debug)]
pub struct Trajectory {
    start: Vec3,
    velocity: Vec3,
}

impl Trajectory {
    /// Траектория, которая через `FLIGHT_TIME` приходит из `start` в `target`.
    pub fn aimed(start: Vec3, target: Vec3) -> Self {
        let fall = 0.5 * gravity() * FLIGHT_TIME * FLIGHT_TIME;
        Self {
            start,
            velocity: (target - start - fall) / FLIGHT_TIME,
        }
    }

    /// Положение снаряда через `t` секунд после выстрела.
    pub fn position(&self, t: f32) -> Vec3 {
        self.start + self.velocity * t + 0.5 * gravity() * t * t
    }
}

/// Вектор гравитации снаряда.
fn gravity() -> Vec3 {
    Vec3::new(0.0, -GRAVITY, 0.0)
}

/// Первое касание поверхности на отрезке времени `(from, to]`: время и точка на поверхности.
///
/// Отрезок делится на шаги не длиннее `HIT_STEP` по пути снаряда, поэтому быстрый снаряд
/// не проскакивает сквозь гребень холма. Точка касания уточняется бинарным поиском.
pub fn first_hit(terrain: &Terrain, path: &Trajectory, from: f32, to: f32) -> Option<(f32, Vec3)> {
    let below = |t: f32| {
        let p = path.position(t);
        p.y <= terrain.height_at(p.x, p.z)
    };
    let length = path.position(from).distance(path.position(to));
    let steps = ((length / HIT_STEP).ceil() as usize).max(1);
    let dt = (to - from) / steps as f32;
    let hit = (1..=steps).find(|&i| below(from + dt * i as f32))?;
    let (mut above_t, mut below_t) = (from + dt * (hit - 1) as f32, from + dt * hit as f32);
    for _ in 0..HIT_REFINE_STEPS {
        let mid = (above_t + below_t) * 0.5;
        if below(mid) {
            below_t = mid;
        } else {
            above_t = mid;
        }
    }
    let p = path.position(below_t);
    Some((below_t, Vec3::new(p.x, terrain.height_at(p.x, p.z), p.z)))
}

/// Куда упадёт снаряд по нынешнему рельефу. Без касания — точка в конце наибольшего времени полёта.
pub fn predict_impact(terrain: &Terrain, path: &Trajectory) -> Vec3 {
    first_hit(terrain, path, ARM_TIME, MAX_FLIGHT_TIME)
        .map_or_else(|| path.position(MAX_FLIGHT_TIME), |(_, p)| p)
}

/// Состояние мортиры: сколько осталось до конца перезарядки.
#[derive(Resource, Default, Debug)]
pub struct Mortar {
    /// Остаток перезарядки, с. Ноль — мортира готова.
    cooldown: f32,
}

impl Mortar {
    /// Отсчитывает перезарядку и решает, стреляет ли мортира в этом кадре при нажатом спуске.
    fn try_fire(&mut self, dt: f32, trigger: bool) -> bool {
        self.cooldown = (self.cooldown - dt).max(0.0);
        if trigger && self.cooldown <= 0.0 {
            self.cooldown = RELOAD_TIME;
            true
        } else {
            false
        }
    }
}

/// Летящий снаряд.
#[derive(Component)]
struct Shell {
    path: Trajectory,
    /// Время с выстрела, с.
    elapsed: f32,
    /// Отметка точки падения этого снаряда.
    marker: Entity,
}

/// Отметка будущей точки падения на земле.
#[derive(Component)]
struct ImpactMarker {
    position: Vec3,
}

/// Вспышка у ствола: свет и светящийся шар, которые гаснут за `FLASH_TIME`.
#[derive(Component, Default)]
struct MuzzleFlash {
    age: f32,
}

/// Меши и материалы мортиры, создаются один раз.
#[derive(Resource)]
struct MortarAssets {
    shell_mesh: Handle<Mesh>,
    shell_material: Handle<StandardMaterial>,
    flash_mesh: Handle<Mesh>,
    flash_material: Handle<StandardMaterial>,
    marker_material: Handle<StandardMaterial>,
}

/// Плагин мортиры.
pub struct MortarPlugin;

impl Plugin for MortarPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mortar>()
            .add_systems(Startup, create_assets)
            .add_systems(
                Update,
                (
                    fire.after(aim::update_aim_point),
                    fly_shells,
                    update_impact_markers,
                    fade_muzzle_flash,
                ),
            );
    }
}

/// Создаёт меши и материалы снаряда, вспышки и отметки падения.
fn create_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(MortarAssets {
        shell_mesh: meshes.add(Sphere::new(SHELL_RADIUS)),
        // Тёмный корпус с оранжевым свечением: снаряд виден и на зелени, и на тёмных склонах.
        shell_material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.15, 0.12, 0.1),
            emissive: LinearRgba::rgb(4.0, 1.6, 0.4),
            ..default()
        }),
        flash_mesh: meshes.add(Sphere::new(1.0)),
        flash_material: materials.add(StandardMaterial {
            base_color: FLASH_COLOR,
            emissive: LinearRgba::rgb(12.0, 7.0, 2.5),
            unlit: true,
            ..default()
        }),
        marker_material: materials.add(StandardMaterial {
            base_color: IMPACT_MARKER_COLOR,
            unlit: true,
            ..default()
        }),
    });
}

/// Стреляет в точку прицела по левой кнопке мыши, если выбрана мортира и кончилась перезарядка.
fn fire(
    mut commands: Commands,
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    weapon: Res<CurrentWeapon>,
    aim: Res<AimPoint>,
    terrain: Res<Terrain>,
    assets: Res<MortarAssets>,
    mut mortar: ResMut<Mortar>,
    mut meshes: ResMut<Assets<Mesh>>,
    rover: Single<(&Transform, &Health), With<Rover>>,
) {
    let (rover, health) = *rover;
    let trigger =
        buttons.pressed(MouseButton::Left) && *weapon == CurrentWeapon::Mortar && !health.is_dead();
    if !mortar.try_fire(time.delta_secs(), trigger) {
        return;
    }
    let muzzle = rover.transform_point(MUZZLE_OFFSET);
    let path = Trajectory::aimed(muzzle, aim.position);
    let impact = predict_impact(&terrain, &path);

    let marker = commands
        .spawn((
            ImpactMarker { position: impact },
            Mesh3d(meshes.add(aim::marker_mesh(|_| 0.0))),
            MeshMaterial3d(assets.marker_material.clone()),
            Transform::from_translation(impact),
            // Вершины ложатся на рельеф каждый кадр, а границы меша нет.
            NoFrustumCulling,
        ))
        .id();
    commands.spawn((
        Shell {
            path,
            elapsed: 0.0,
            marker,
        },
        Mesh3d(assets.shell_mesh.clone()),
        MeshMaterial3d(assets.shell_material.clone()),
        Transform::from_translation(muzzle),
    ));
    commands.spawn((
        MuzzleFlash::default(),
        Mesh3d(assets.flash_mesh.clone()),
        MeshMaterial3d(assets.flash_material.clone()),
        Transform::from_translation(muzzle).with_scale(Vec3::splat(FLASH_RADIUS)),
        PointLight {
            color: FLASH_COLOR,
            intensity: FLASH_INTENSITY,
            range: 20.0,
            shadow_maps_enabled: false,
            ..default()
        },
    ));
}

/// Двигает снаряды по траектории и взрывает их при первом касании поверхности.
fn fly_shells(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut explosions: MessageWriter<Explosion>,
    mut shells: Query<(Entity, &mut Shell, &mut Transform)>,
) {
    for (entity, mut shell, mut transform) in &mut shells {
        let from = shell.elapsed.max(ARM_TIME);
        shell.elapsed += time.delta_secs();
        let to = shell.elapsed;
        let hit = if to > from {
            first_hit(&terrain, &shell.path, from, to).map(|(_, p)| p)
        } else {
            None
        };
        let hit = hit.or_else(|| (to >= MAX_FLIGHT_TIME).then(|| shell.path.position(to)));
        if let Some(position) = hit {
            explosions.write(Explosion {
                position,
                radius: EXPLOSION_RADIUS,
                power: EXPLOSION_POWER,
                source_team: Team::Player,
            });
            commands.entity(entity).despawn();
            commands.entity(shell.marker).despawn();
            continue;
        }
        transform.translation = shell.path.position(to);
    }
}

/// Кладёт вершины отметок падения на рельеф: под отметкой рельеф может измениться.
fn update_impact_markers(
    terrain: Res<Terrain>,
    mut meshes: ResMut<Assets<Mesh>>,
    markers: Query<(&ImpactMarker, &Mesh3d)>,
) {
    for (marker, mesh) in &markers {
        let center = marker.position;
        if let Some(mut mesh) = meshes.get_mut(&mesh.0) {
            *mesh = aim::marker_mesh(|offset| {
                terrain.height_at(center.x + offset.x, center.z + offset.y) - center.y
                    + aim::MARKER_LIFT
                    + IMPACT_MARKER_EXTRA_LIFT
            });
        }
    }
}

/// Гасит вспышку у ствола и удаляет её.
fn fade_muzzle_flash(
    mut commands: Commands,
    time: Res<Time>,
    mut flashes: Query<(Entity, &mut MuzzleFlash, &mut Transform, &mut PointLight)>,
) {
    for (entity, mut flash, mut transform, mut light) in &mut flashes {
        flash.age += time.delta_secs();
        let left = 1.0 - flash.age / FLASH_TIME;
        if left <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        light.intensity = FLASH_INTENSITY * left * left;
        transform.scale = Vec3::splat(FLASH_RADIUS * (0.4 + 0.6 * left));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::ChunkData;

    #[test]
    fn shell_reaches_target_after_flight_time() {
        let start = Vec3::new(1.0, 3.0, -2.0);
        for target in [Vec3::new(30.0, -4.0, 12.0), Vec3::new(-5.0, 9.0, 1.0)] {
            let path = Trajectory::aimed(start, target);
            assert!(path.position(0.0).distance(start) < 1e-5);
            assert!(path.position(FLIGHT_TIME).distance(target) < 1e-3);
        }
    }

    #[test]
    fn shell_lands_on_aim_point() {
        let mut terrain = Terrain::new(11);
        terrain.chunk(IVec2::ZERO);
        let start = Vec3::new(4.0, terrain.height_at(4.0, 4.0) + 1.0, 4.0);
        let target = Vec3::new(28.0, terrain.height_at(28.0, 20.0), 20.0);
        let impact = predict_impact(&terrain, &Trajectory::aimed(start, target));
        assert!(
            impact.distance(target) < 0.1,
            "упал в {impact}, цель {target}"
        );
    }

    #[test]
    fn shell_flies_over_hill() {
        let mut terrain = Terrain::new(5);
        let chunk = terrain.chunk_mut(IVec2::ZERO);
        // Ровная площадка и холм высотой 6 м поперёк пути посередине.
        for iz in 0..crate::terrain::CHUNK_VERTS {
            for ix in 0..crate::terrain::CHUNK_VERTS {
                let hill = if (14..=18).contains(&ix) { 6.0 } else { 0.0 };
                chunk.heights[ChunkData::index(ix, iz)] = hill;
            }
        }
        let start = Vec3::new(2.0, 1.0, 16.0);
        let target = Vec3::new(30.0, 0.0, 16.0);
        let impact = predict_impact(&terrain, &Trajectory::aimed(start, target));
        assert!(
            impact.distance(target) < 0.1,
            "упал в {impact}, цель {target}"
        );
    }

    #[test]
    fn shell_hits_wall_before_target() {
        let mut terrain = Terrain::new(5);
        let chunk = terrain.chunk_mut(IVec2::ZERO);
        // Стена высотой 30 м перед целью: дуга её не перелетает.
        for iz in 0..crate::terrain::CHUNK_VERTS {
            for ix in 0..crate::terrain::CHUNK_VERTS {
                let wall = if (20..=22).contains(&ix) { 30.0 } else { 0.0 };
                chunk.heights[ChunkData::index(ix, iz)] = wall;
            }
        }
        let start = Vec3::new(2.0, 1.0, 16.0);
        let target = Vec3::new(30.0, 0.0, 16.0);
        let impact = predict_impact(&terrain, &Trajectory::aimed(start, target));
        assert!(impact.x < 22.0, "снаряд прошёл сквозь стену: {impact}");
        assert!((impact.y - terrain.height_at(impact.x, impact.z)).abs() < 1e-3);
    }

    #[test]
    fn reload_limits_rate_of_fire() {
        let mut mortar = Mortar::default();
        let dt = 1.0 / 60.0;
        // Кнопка зажата 3,5 с: выстрел сразу и затем раз в секунду.
        let shots = (0..210).filter(|_| mortar.try_fire(dt, true)).count();
        assert_eq!(shots, 4);
        // Кнопка отпущена: перезарядка дошла, следующий клик стреляет сразу.
        mortar.try_fire(1.0, false);
        assert!(mortar.try_fire(dt, true));
        assert!(!mortar.try_fire(dt, true));
    }
}
