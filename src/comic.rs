//! Комикс-надписи оружия: «BOMBARDIRO!», «TUNG TUNG TUNG», «WOMP WOMP», «BRUH».
//!
//! Надпись вылетает у места события, раздувается и тает за 0,8 с. Мем редкий: у пулемёта —
//! каждая 25-я пуля, у лазера — каждый 6-й импульс, у мощного взрыва — по шансу, у пустого оружия —
//! всегда; одна и та же надпись не чаще раза в 8 с.

use bevy::prelude::*;

use crate::arsenal::WeaponDenied;
use crate::bullet::FireBullet;
use crate::camera::GameCamera;
use crate::combat::Team;
use crate::explosion::Explosion;
use crate::laser::FireLaser;
use crate::rover::Rover;
use crate::run::{RunScoped, RunStarted};
use crate::sound_list::Dice;

/// Сколько живёт надпись, с.
const LIFETIME: f32 = 0.8;
/// Одна и та же надпись не чаще, с.
const COOLDOWN: f32 = 8.0;
/// Каждая какая пуля пулемёта и импульс лазера дают надпись.
const BULLET_EVERY: u32 = 25;
const LASER_EVERY: u32 = 6;

/// Надписи и их цвета.
const CAPTIONS: [(&str, Color); 6] = [
    ("BOMBARDIRO!", Color::srgb(1.0, 0.55, 0.1)),
    ("KA-BOOM!", Color::srgb(1.0, 0.85, 0.2)),
    ("TUNG TUNG TUNG", Color::srgb(0.6, 0.95, 1.0)),
    ("WOMP WOMP", Color::srgb(0.7, 0.9, 1.0)),
    ("BRUH", Color::srgb(0.9, 0.9, 0.9)),
    ("6-7", Color::srgb(1.0, 0.4, 0.8)),
];

/// Плагин комикс-надписей.
pub struct ComicPlugin;

impl Plugin for ComicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ComicState>()
            .add_systems(Update, (pick_captions, float_captions).chain());
    }
}

/// Счётчики выстрелов и время до следующего показа каждой надписи.
#[derive(Resource, Default)]
struct ComicState {
    bullets: u32,
    lasers: u32,
    cooldowns: [f32; CAPTIONS.len()],
}

/// Вылетевшая надпись: точка мира и возраст.
#[derive(Component)]
struct Caption {
    world: Vec3,
    age: f32,
    drift: f32,
}

/// Выбирает поводы для надписей и создаёт их.
#[allow(clippy::too_many_arguments)]
fn pick_captions(
    mut commands: Commands,
    time: Res<Time>,
    mut state: ResMut<ComicState>,
    mut dice: ResMut<Dice>,
    mut started: MessageReader<RunStarted>,
    mut explosions: MessageReader<Explosion>,
    mut bullets: MessageReader<FireBullet>,
    mut lasers: MessageReader<FireLaser>,
    mut denied: MessageReader<WeaponDenied>,
    rover: Query<&Transform, With<Rover>>,
) {
    if started.read().count() > 0 {
        *state = ComicState::default();
    }
    let dt = time.delta_secs();
    for c in &mut state.cooldowns {
        *c = (*c - dt).max(0.0);
    }
    let mut wanted: Vec<(usize, Vec3)> = Vec::new();
    for e in explosions.read() {
        if e.source_team == Team::Player && e.power >= 1.3 && dice.range(0.0, 1.0) < 0.35 {
            let index = if dice.range(0.0, 1.0) < 0.6 { 0 } else { 1 };
            wanted.push((index, e.position + Vec3::Y * 3.0));
        }
    }
    for b in bullets.read() {
        if b.team == Team::Player {
            state.bullets += 1;
            if state.bullets.is_multiple_of(BULLET_EVERY) {
                let index = if dice.range(0.0, 1.0) < 0.75 { 2 } else { 5 };
                wanted.push((index, b.origin + Vec3::Y * 1.5));
            }
        }
    }
    for l in lasers.read() {
        if l.team == Team::Player {
            state.lasers += 1;
            if state.lasers.is_multiple_of(LASER_EVERY) {
                wanted.push((3, l.origin + Vec3::Y * 1.5));
            }
        }
    }
    let rover = rover.iter().next().map(|t| t.translation);
    for _ in denied.read() {
        if let Some(p) = rover {
            wanted.push((4, p + Vec3::Y * 2.0));
        }
    }
    for (index, world) in wanted {
        if state.cooldowns[index] > 0.0 {
            continue;
        }
        state.cooldowns[index] = COOLDOWN;
        let (text, color) = CAPTIONS[index];
        commands.spawn((
            Caption {
                world,
                age: 0.0,
                drift: dice.range(-30.0, 30.0),
            },
            RunScoped,
            Text::new(text),
            TextFont {
                font_size: FontSize::Px(44.0),
                ..default()
            },
            TextColor(color),
            TextShadow::default(),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            UiTransform::default(),
        ));
    }
}

/// Надпись держится у своей точки мира, раздувается с отскоком и тает.
fn float_captions(
    mut commands: Commands,
    time: Res<Time>,
    camera: Query<(&Camera, &GlobalTransform), With<GameCamera>>,
    mut captions: Query<(Entity, &mut Caption, &mut Node, &mut TextColor, &mut UiTransform, &ComputedNode)>,
) {
    let Some((camera, camera_transform)) = camera.iter().next() else {
        return;
    };
    for (entity, mut caption, mut node, mut color, mut transform, computed) in &mut captions {
        caption.age += time.delta_secs();
        let t = caption.age / LIFETIME;
        if t >= 1.0 {
            commands.entity(entity).despawn();
            continue;
        }
        if let Ok(screen) = camera.world_to_viewport(camera_transform, caption.world) {
            let size = computed.size() * computed.inverse_scale_factor();
            node.left = Val::Px(screen.x - size.x * 0.5 + caption.drift * t);
            node.top = Val::Px(screen.y - size.y * 0.5 - 40.0 * t);
        }
        // Выскакивает с перелётом, затем медленно растёт.
        let pop = if t < 0.2 { 0.4 + 4.0 * t } else { 1.2 - 0.2 * ((t - 0.2) / 0.8) + 0.15 * t };
        transform.scale = Vec2::splat(pop);
        transform.rotation = Rot2::degrees(caption.drift * 0.2);
        color.0.set_alpha(if t > 0.6 { 1.0 - (t - 0.6) / 0.4 } else { 1.0 });
    }
}
