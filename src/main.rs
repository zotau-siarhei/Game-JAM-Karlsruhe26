//! Lunokhod: луноход попадает на неизвестную планету и отбивается от врагов.
//!
//! Пока это заглушка: окно, поверхность планеты, луноход-кубик, свет и камера.

use bevy::prelude::*;

/// Цвет неба над планетой.
const SKY_COLOR: Color = Color::srgb(0.02, 0.02, 0.06);

/// Цвет грунта неизвестной планеты.
const GROUND_COLOR: Color = Color::srgb(0.55, 0.32, 0.22);

/// Цвет корпуса лунохода.
const ROVER_COLOR: Color = Color::srgb(0.75, 0.78, 0.82);

/// Размер стороны квадратной поверхности планеты.
const GROUND_SIZE: f32 = 40.0;

/// Маркер лунохода, которым управляет игрок.
#[derive(Component)]
struct Rover;

fn main() {
    App::new()
        .insert_resource(ClearColor(SKY_COLOR))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Lunokhod".into(),
                ..default()
            }),
            ..default()
        }))
        .add_systems(Startup, setup)
        .run();
}

/// Создаёт стартовую сцену.
fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(GROUND_SIZE, GROUND_SIZE))),
        MeshMaterial3d(materials.add(GROUND_COLOR)),
    ));

    commands.spawn((
        Rover,
        Mesh3d(meshes.add(Cuboid::new(1.2, 0.6, 1.8))),
        MeshMaterial3d(materials.add(ROVER_COLOR)),
        Transform::from_xyz(0.0, 0.3, 0.0),
    ));

    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(8.0, 12.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 9.0, 12.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}
