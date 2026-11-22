//! Скелетная анимация жителей-людей: стоит, идёт, бежит, говорит с жестами.
//!
//! Модели людей (`resident_*` в `assets/models/models.json`) сделаны в Meshy со скелетом
//! Mixamo и четырьмя клипами на месте, без ухода корня. Здесь из клипов модели строится граф
//! анимаций, он вешается на `AnimationPlayer`, который загрузчик glTF ставит в сцену модели, и
//! клип выбирается по тому, как житель движется. Темп клипа подгоняется под скорость жителя,
//! чтобы ступни не скользили по земле.
//!
//! Чтобы на экране не бегали клоны, у каждого жителя свой скин (перекрашенная текстура из
//! `assets/models/skins`, делает `scripts/recolor_people.py`), свой темп шага и своя фаза клипа.

use std::collections::HashMap;
use std::time::Duration;

use bevy::gltf::Gltf;
use bevy::prelude::*;

use crate::frame::GameSet;
use crate::models::Models;
use crate::residents::Resident;

/// Имена клипов в моделях людей.
const IDLE_CLIP: &str = "Idle_02";
const WALK_CLIP: &str = "Walking";
const RUN_CLIP: &str = "Running";
const TALK_CLIP: &str = "Talk_with_Hands_Open";

/// С какой скоростью тело идёт в клипе ходьбы и бега при темпе 1, м/с: скорость ступни в фазе
/// опоры, посчитанная по скелету моделей (у всех четырёх 1,23–1,25 и 4,0–4,4).
const CLIP_WALK_SPEED: f32 = 1.24;
const CLIP_RUN_SPEED: f32 = 4.1;

/// Ниже этой скорости житель стоит, м/с.
const STAND_SPEED: f32 = 0.25;
/// Выше этой скорости шаг переходит в бег, м/с.
const RUN_FROM: f32 = 2.6;
/// Пределы темпа клипа: дальше шаг выглядит неестественно.
const RATE_LIMITS: (f32, f32) = (0.6, 1.6);
/// Сглаживание скорости, 1/с: скорость берётся по смещению за кадр и не дёргается.
const SPEED_SMOOTHING: f32 = 10.0;
/// Длительность перехода между клипами, с.
const BLEND: f32 = 0.25;

/// Плагин скелетной анимации жителей.
pub struct ResidentAnimPlugin;

impl Plugin for ResidentAnimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PeopleClips>()
            .init_resource::<SkinMaterials>()
            .add_systems(
                Update,
                (build_graphs, attach_players, drive_animation)
                    .chain()
                    .in_set(GameSet::Presentation),
            );
    }
}

/// Модель человека, которую получил житель, и номер скина (0 — текстура из файла модели).
#[derive(Component, Clone, Copy)]
pub struct ResidentModel(pub &'static str, pub u8);

/// Аниматор жителя: сущность с `AnimationPlayer` в сцене его модели и сглаженная скорость.
#[derive(Component)]
pub struct Animator {
    player: Entity,
    last: Option<Vec3>,
    speed: f32,
    /// Своя фаза клипа от 0 до 1: соседи, испуганные одним взрывом, не шагают в ногу.
    phase: f32,
}

/// Длительность клипов моделей людей, с: у всех четырёх одинаковые наборы из Meshy.
fn clip_length(gait: Gait) -> f32 {
    match gait {
        Gait::Stand => 1.867,
        Gait::Talk => 3.967,
        Gait::Walk => 1.033,
        Gait::Run => 0.633,
    }
}

/// Материалы скинов: материал модели с перекрашенной текстурой, по модели и номеру скина.
#[derive(Resource, Default)]
struct SkinMaterials(HashMap<(&'static str, u8), Handle<StandardMaterial>>);

/// Что делает житель — от этого зависит клип.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Gait {
    Stand,
    Talk,
    Walk,
    Run,
}

/// Граф анимаций модели и узлы её клипов.
struct Clips {
    graph: Handle<AnimationGraph>,
    idle: AnimationNodeIndex,
    walk: AnimationNodeIndex,
    run: AnimationNodeIndex,
    talk: AnimationNodeIndex,
}

impl Clips {
    fn node(&self, gait: Gait) -> AnimationNodeIndex {
        match gait {
            Gait::Stand => self.idle,
            Gait::Talk => self.talk,
            Gait::Walk => self.walk,
            Gait::Run => self.run,
        }
    }
}

/// Графы анимаций по моделям: строятся один раз, когда загружен файл модели.
#[derive(Resource, Default)]
struct PeopleClips(HashMap<&'static str, Clips>);

/// Клип и его темп по скорости жителя и его занятию.
fn gait(speed: f32, running: bool, chatting: bool) -> (Gait, f32) {
    let rate = |natural: f32| (speed / natural).clamp(RATE_LIMITS.0, RATE_LIMITS.1);
    if speed < STAND_SPEED {
        (if chatting { Gait::Talk } else { Gait::Stand }, 1.0)
    } else if running || speed > RUN_FROM {
        (Gait::Run, rate(CLIP_RUN_SPEED))
    } else {
        (Gait::Walk, rate(CLIP_WALK_SPEED))
    }
}

/// Строит граф анимаций для каждой модели людей, как только её файл загружен.
fn build_graphs(
    mut clips: ResMut<PeopleClips>,
    models: Models,
    residents: Query<&ResidentModel, Without<Animator>>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    for &ResidentModel(id, _) in &residents {
        if clips.0.contains_key(id) {
            continue;
        }
        let Some(gltf) = models.gltf(id).and_then(|h| gltfs.get(&h)) else {
            continue;
        };
        let clip = |name: &str| gltf.named_animations.get(name).cloned();
        let (Some(idle), Some(walk), Some(run), Some(talk)) = (
            clip(IDLE_CLIP),
            clip(WALK_CLIP),
            clip(RUN_CLIP),
            clip(TALK_CLIP),
        ) else {
            warn_once!(
                "Жители: в модели «{id}» нет клипов {IDLE_CLIP}, {WALK_CLIP}, {RUN_CLIP}, {TALK_CLIP}"
            );
            continue;
        };
        let (graph, nodes) = AnimationGraph::from_clips([idle, walk, run, talk]);
        clips.0.insert(
            id,
            Clips {
                graph: graphs.add(graph),
                idle: nodes[0],
                walk: nodes[1],
                run: nodes[2],
                talk: nodes[3],
            },
        );
    }
}

/// Находит `AnimationPlayer` в сцене модели жителя, вешает граф, запускает стояние со своей фазой
/// и надевает скин.
#[allow(clippy::too_many_arguments)]
fn attach_players(
    mut commands: Commands,
    clips: Res<PeopleClips>,
    mut skins: ResMut<SkinMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    players: Query<Entity, (With<AnimationPlayer>, Without<AnimationGraphHandle>, Without<crate::anim_lod::ParkedGraph>)>,
    parents: Query<&ChildOf>,
    children: Query<&Children>,
    // Значок тревоги над головой — тоже потомок жителя, но скин ему не надевается.
    mut meshes: Query<&mut MeshMaterial3d<StandardMaterial>, Without<crate::residents::Alert>>,
    residents: Query<&ResidentModel, Without<Animator>>,
) {
    for player in &players {
        // Вверх по иерархии до жителя: сцена модели — его потомок.
        let Some((resident, look)) = parents
            .iter_ancestors(player)
            .find_map(|e| residents.get(e).ok().map(|m| (e, *m)))
        else {
            continue;
        };
        let ResidentModel(model, skin) = look;
        let Some(clips) = clips.0.get(model) else {
            continue;
        };
        // Фаза от номера сущности: своя у каждого и одна и та же между кадрами.
        let phase = (resident.index().index() as f32 * 0.618_034).fract();
        let mut transitions = AnimationTransitions::new();
        let mut animation = AnimationPlayer::default();
        transitions
            .play(&mut animation, clips.idle, Duration::ZERO)
            .repeat()
            .seek_to(phase * clip_length(Gait::Stand));
        commands.entity(player).insert((
            AnimationGraphHandle(clips.graph.clone()),
            transitions,
            animation,
        ));
        commands.entity(resident).insert(Animator {
            player,
            last: None,
            speed: 0.0,
            phase,
        });

        if skin == 0 {
            continue;
        }
        for mesh in children.iter_descendants(resident) {
            let Ok(mut material) = meshes.get_mut(mesh) else {
                continue;
            };
            let handle = skins
                .0
                .entry((model, skin))
                .or_insert_with(|| {
                    // Материал модели живёт только в рендер-мире (`models::texture_in_gpu_only`),
                    // скопировать его нельзя: свойства — как у материала жителей в glTF.
                    // Иначе материал выходит односторонним и глянцевым, а его конвейер — поздним.
                    materials.add(StandardMaterial {
                        base_color_texture: Some(asset_server.load(format!("models/skins/{model}_{skin}.png"))),
                        perceptual_roughness: 1.0,
                        metallic: 0.0,
                        double_sided: true,
                        cull_mode: None,
                        ..default()
                    })
                })
                .clone();
            material.0 = handle;
        }
    }
}

/// Выбирает клип по движению жителя и подгоняет его темп под скорость.
fn drive_animation(
    time: Res<Time>,
    clips: Res<PeopleClips>,
    mut residents: Query<(&Resident, &ResidentModel, &Transform, &mut Animator)>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (resident, model, transform, mut animator) in &mut residents {
        let here = transform.translation;
        let step = animator
            .last
            .map_or(0.0, |last| (here - last).with_y(0.0).length());
        animator.last = Some(here);
        // Перенос (возвращение в посёлок, выход из дома) — не шаг.
        let speed = if step / dt > 12.0 { 0.0 } else { step / dt };
        animator.speed += (speed - animator.speed) * (SPEED_SMOOTHING * dt).min(1.0);

        let (Some(clips), Ok((mut player, mut transitions))) =
            (clips.0.get(model.0), players.get_mut(animator.player))
        else {
            continue;
        };
        let (gait, rate) = gait(
            animator.speed,
            resident.is_running(),
            resident.is_chatting(),
        );
        let node = clips.node(gait);
        if transitions.get_main_animation() != Some(node) {
            transitions
                .play(&mut player, node, Duration::from_secs_f32(BLEND))
                .repeat()
                .seek_to(animator.phase * clip_length(gait));
        }
        if let Some(active) = player.animation_mut(node) {
            active.set_speed(rate);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn стоит_идёт_бежит_по_скорости() {
        assert_eq!(gait(0.0, false, false).0, Gait::Stand);
        assert_eq!(gait(0.0, false, true).0, Gait::Talk);
        assert_eq!(gait(1.6, false, false).0, Gait::Walk);
        assert_eq!(gait(1.6, true, false).0, Gait::Run);
        assert_eq!(gait(4.5, false, false).0, Gait::Run);
    }

    #[test]
    fn темп_клипа_совпадает_со_скоростью_тела() {
        let (_, walk) = gait(1.6, false, false);
        assert!((walk * CLIP_WALK_SPEED - 1.6).abs() < 1e-4);
        let (_, run) = gait(4.5, true, false);
        assert!((run * CLIP_RUN_SPEED - 4.5).abs() < 1e-4);
        // Медленный бег на месте не замирает.
        assert_eq!(gait(0.5, true, false).1, RATE_LIMITS.0);
    }
}
