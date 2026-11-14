//! Звуковые эффекты.
//!
//! Список звуков лежит в `assets/sfx/sounds.json`, файлы — рядом с ним. Чтобы проиграть звук,
//! отправь сообщение [`PlaySound`]:
//!
//! ```ignore
//! fn on_explosion(mut sounds: MessageWriter<PlaySound>) {
//!     sounds.write(PlaySound::at("explosion", position));
//! }
//! ```
//!
//! Идентификатор — основа группы: `explosion` выбирает случайный из `explosion`, `explosion_1`,
//! `explosion_2`. Громкость убывает с расстоянием до слушателя — сущности с [`SoundListener`].

use std::collections::HashSet;

use bevy::prelude::*;

use crate::rover::Rover;
use crate::sound_list::{Dice, SoundList};

/// Каталог звуков внутри `assets`.
const SFX_DIR: &str = "sfx";

/// Файл со списком звуков внутри [`SFX_DIR`].
const LIST_FILE: &str = "sounds.json";

/// Расстояние в метрах, на котором громкость падает вдвое.
const HALF_VOLUME_DISTANCE: f32 = 25.0;

/// Дальше этого расстояния звук не создаётся.
const MAX_DISTANCE: f32 = 90.0;

/// Разброс скорости воспроизведения, чтобы частые звуки не повторялись один в один.
const SPEED_SPREAD: (f32, f32) = (0.9, 1.1);

/// Сколько экземпляров одной группы звучит одновременно, если для группы нет своего предела.
const DEFAULT_LIMIT: usize = 4;

/// Свои пределы одновременных экземпляров по началу идентификатора группы.
const LIMITS: &[(&str, usize)] = &[
    ("mg_", 6),
    ("explosion", 5),
    ("building_", 3),
    ("resident_", 4),
    ("ui_", 2),
];

/// Подключает загрузку звуков и их воспроизведение по сообщению [`PlaySound`].
pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Sounds>()
            .init_resource::<Dice>()
            .add_message::<PlaySound>()
            .add_systems(Startup, load_sounds)
            .add_systems(Update, (attach_listener, play_sounds));
    }
}

/// Просьба проиграть звук.
#[derive(Message, Debug, Clone)]
pub struct PlaySound {
    /// Основа группы звуков или идентификатор конкретной записи.
    pub id: String,
    /// Место события в мире. `None` — звук без расстояния: интерфейс, сам луноход.
    pub position: Option<Vec3>,
}

impl PlaySound {
    // Игровых событий, которые отправляют звуки, пока нет.
    #[allow(dead_code)]
    /// Звук без места в мире: всегда на полной громкости.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            position: None,
        }
    }

    #[allow(dead_code)]
    /// Звук события в точке мира: громкость убывает с расстоянием до слушателя.
    pub fn at(id: impl Into<String>, position: Vec3) -> Self {
        Self {
            id: id.into(),
            position: Some(position),
        }
    }
}

/// Метка слушателя: от этой сущности считается расстояние до звуков. Ставится на луноход.
#[derive(Component)]
pub struct SoundListener;

/// Делает слушателем луноход, как только он появился.
fn attach_listener(mut commands: Commands, rovers: Query<Entity, Added<Rover>>) {
    for rover in &rovers {
        commands.entity(rover).insert(SoundListener);
    }
}

/// Все звуки из `sounds.json`.
#[derive(Resource, Default)]
struct Sounds {
    list: SoundList,
}

/// Звучащий экземпляр звука и его группа, для предела одновременных экземпляров.
#[derive(Component)]
struct SoundInstance {
    group: String,
}

fn load_sounds(mut sounds: ResMut<Sounds>, asset_server: Res<AssetServer>) {
    sounds.list = SoundList::load(&asset_server, SFX_DIR, LIST_FILE, "Звуки");
}

/// Громкость звука на расстоянии `distance` метров от слушателя. `None` — слишком далеко.
fn volume_at(distance: f32) -> Option<f32> {
    if distance > MAX_DISTANCE {
        return None;
    }
    let ratio = distance / HALF_VOLUME_DISTANCE;
    Some(1.0 / (1.0 + ratio * ratio))
}

/// Предел одновременных экземпляров для группы.
fn limit_for(group: &str) -> usize {
    LIMITS
        .iter()
        .find(|(prefix, _)| group.starts_with(prefix))
        .map_or(DEFAULT_LIMIT, |(_, limit)| *limit)
}

fn play_sounds(
    mut commands: Commands,
    mut requests: MessageReader<PlaySound>,
    sounds: Res<Sounds>,
    mut dice: ResMut<Dice>,
    time: Res<Time<Virtual>>,
    listener: Query<&GlobalTransform, With<SoundListener>>,
    instances: Query<&SoundInstance>,
    mut reported: Local<HashSet<String>>,
) {
    if requests.is_empty() {
        return;
    }
    let listener_position = listener.iter().next().map(GlobalTransform::translation);
    // Сколько экземпляров каждой группы уже звучит, вместе с созданными в этом кадре.
    let mut started: Vec<String> = Vec::new();

    for request in requests.read() {
        let volume = match (request.position, listener_position) {
            (Some(position), Some(listener)) => {
                let Some(volume) = volume_at(position.distance(listener)) else {
                    continue;
                };
                volume
            }
            _ => 1.0,
        };

        let variants: Vec<_> = sounds.list.group(&request.id).collect();
        let Some(chosen) = variants.get(dice.below(variants.len())) else {
            // О каждом идентификаторе пишем один раз, иначе пулемёт забьёт лог.
            if reported.insert(request.id.clone()) {
                if sounds.list.has_group(&request.id) {
                    warn!(
                        "Звуки: у «{}» нет звукового файла в assets/{SFX_DIR}",
                        request.id
                    );
                } else {
                    warn!(
                        "Звуки: «{}» нет в {SFX_DIR}/{LIST_FILE} — опечатка в идентификаторе?",
                        request.id
                    );
                }
            }
            continue;
        };

        let active = instances
            .iter()
            .filter(|instance| instance.group == request.id)
            .count()
            + started.iter().filter(|group| **group == request.id).count();
        if active >= limit_for(&request.id) {
            continue;
        }
        started.push(request.id.clone());

        let Some(sound) = &chosen.sound else {
            continue;
        };
        // Замедление времени растягивает звук.
        let speed = dice.range(SPEED_SPREAD.0, SPEED_SPREAD.1) * time.relative_speed().max(0.1);
        commands.spawn((
            SoundInstance {
                group: request.id.clone(),
            },
            AudioPlayer::new(sound.clone()),
            PlaybackSettings {
                volume: bevy::audio::Volume::Linear(volume),
                speed,
                ..PlaybackSettings::DESPAWN
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_falls_with_distance() {
        assert_eq!(volume_at(0.0), Some(1.0));
        let near = volume_at(10.0).unwrap();
        let far = volume_at(60.0).unwrap();
        assert!(near > far);
        assert!((volume_at(HALF_VOLUME_DISTANCE).unwrap() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn too_far_sound_is_not_played() {
        assert_eq!(volume_at(MAX_DISTANCE + 1.0), None);
    }

    #[test]
    fn groups_have_limits() {
        assert_eq!(limit_for("mg_fire"), 6);
        assert_eq!(limit_for("explosion_big"), 5);
        assert_eq!(limit_for("pickup"), DEFAULT_LIMIT);
    }
}
