//! Ступени качества картинки: свечение, тени и сглаживание. Переключаются клавишей Q на паузе
//! и в меню настроек, выбор сохраняется вместе с остальными настройками (`settings.rs`).
//!
//! | Ступень | Свечение | Тени | Сглаживание |
//! |---|---|---|---|
//! | Низкая | нет | нет | нет |
//! | Средняя | есть | карта 1024, два каскада до 70 м | нет |
//! | Высокая | есть | карта 2048, три каскада до 110 м | MSAA 4× |
//!
//! Постоянные эффекты лунохода (пыль, дым) берут долю частиц ступени: 40, 70 и 100 %.
//!
//! На всех ступенях камера в HDR с тональной коррекцией: огонь, вспышки и лазер ярче единицы
//! и сияют с ореолом через свечение. Рассеянный свет — голубоватый от неба.

use bevy::light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::{Msaa, Tonemapping};

use crate::camera::GameCamera;
use crate::run::GameState;
use crate::settings::Settings;

/// Ступень качества. В файле настроек — `low`, `medium`, `high`.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Low,
    #[default]
    Medium,
    High,
}

impl Quality {
    pub fn name(self) -> &'static str {
        match self {
            Quality::Low => "low",
            Quality::Medium => "medium",
            Quality::High => "high",
        }
    }

    /// Доля частиц постоянных эффектов на этой ступени: бюджет из `visual-style`.
    pub fn particle_share(self) -> f32 {
        match self {
            Quality::Low => 0.4,
            Quality::Medium => 0.7,
            Quality::High => 1.0,
        }
    }

    /// Следующая ступень по кругу.
    pub fn next(self) -> Self {
        self.step(1)
    }

    /// Соседняя ступень на шаг `step` по кругу.
    pub fn step(self, step: i32) -> Self {
        const ALL: [Quality; 3] = [Quality::Low, Quality::Medium, Quality::High];
        let index = ALL.iter().position(|q| *q == self).unwrap_or(0) as i32;
        ALL[(index + step).rem_euclid(ALL.len() as i32) as usize]
    }
}

/// Плагин ступеней качества.
pub struct QualityPlugin;

impl Plugin for QualityPlugin {
    fn build(&self, app: &mut App) {
        let quality = crate::settings::stored(app).quality;
        app.insert_resource(quality)
            .insert_resource(GlobalAmbientLight {
                color: Color::srgb(0.7, 0.8, 1.0),
                brightness: 220.0,
                affects_lightmapped_meshes: true,
            })
            .add_systems(PostStartup, (setup_camera.after(crate::camera::spawn_camera), spawn_label))
            .add_systems(
                Update,
                (
                    switch_quality.run_if(in_state(GameState::Paused)),
                    (apply_quality, remember_quality).run_if(resource_changed::<Quality>),
                    show_label,
                )
                    .chain()
                    .in_set(crate::frame::GameSet::Presentation),
            );
    }
}

/// Надпись о ступени на паузе.
#[derive(Component)]
struct QualityLabel;

/// Камера в HDR с тональной коррекцией.
fn setup_camera(mut commands: Commands, camera: Single<Entity, With<GameCamera>>) {
    commands
        .entity(*camera)
        .insert((bevy::camera::Hdr, Tonemapping::TonyMcMapface));
}

/// Q на паузе меняет ступень по кругу.
fn switch_quality(keys: Res<ButtonInput<KeyCode>>, mut quality: ResMut<Quality>) {
    if keys.just_pressed(KeyCode::KeyQ) {
        *quality = quality.next();
    }
}

/// Новая ступень — в настройки: они сохраняют её в файл.
fn remember_quality(quality: Res<Quality>, mut settings: ResMut<Settings>) {
    if settings.quality != *quality {
        settings.quality = *quality;
    }
}

/// Применяет ступень к камере, солнцу и карте теней. Тени настраиваются только здесь:
/// солнце создаётся в `main.rs` без них и получает их в первом же кадре.
fn apply_quality(
    mut commands: Commands,
    quality: Res<Quality>,
    camera: Query<Entity, With<GameCamera>>,
    mut suns: Query<(Entity, &mut DirectionalLight)>,
) {
    let q = *quality;
    for camera in &camera {
        let mut c = commands.entity(camera);
        match q {
            Quality::Low => {
                c.remove::<Bloom>().insert(Msaa::Off);
            }
            Quality::Medium => {
                c.insert((Bloom::NATURAL, Msaa::Off));
            }
            Quality::High => {
                c.insert((Bloom::NATURAL, Msaa::Sample4));
            }
        }
    }
    let (size, cascades, far) = match q {
        Quality::Low => (512, 1, 40.0),
        Quality::Medium => (1024, 2, 70.0),
        Quality::High => (2048, 3, 110.0),
    };
    commands.insert_resource(DirectionalLightShadowMap { size });
    for (entity, mut sun) in &mut suns {
        sun.shadow_maps_enabled = q != Quality::Low;
        commands.entity(entity).insert(
            CascadeShadowConfigBuilder {
                num_cascades: cascades,
                first_cascade_far_bound: 30.0,
                maximum_distance: far,
                ..default()
            }
            .build(),
        );
    }
}

fn spawn_label(mut commands: Commands) {
    commands.spawn((
        QualityLabel,
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(20.0),
            ..default()
        },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.8)),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(16.0),
            bottom: Val::Px(16.0),
            ..default()
        },
        Visibility::Hidden,
    ));
}

/// На паузе внизу слева видна текущая ступень и подсказка.
fn show_label(
    quality: Res<Quality>,
    device: Res<crate::gamepad::InputDevice>,
    style: Res<crate::gamepad::PadStyle>,
    state: Res<State<GameState>>,
    mut labels: Query<(&mut Text, &mut Visibility), With<QualityLabel>>,
) {
    let paused = *state.get() == GameState::Paused;
    for (mut text, mut visibility) in &mut labels {
        let wanted = if paused { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
        let key = crate::gamepad::hint_text(*device, *style, "Q", "{SELECT}");
        // При касаниях клавиши нет: ступень меняют в настройках.
        let line = if key.is_empty() {
            format!("QUALITY: {}", quality.name().to_uppercase())
        } else {
            format!("QUALITY: {} ({key})", quality.name().to_uppercase())
        };
        if text.0 != line {
            text.0 = line;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ступени_идут_по_кругу_и_читаются_по_имени() {
        assert_eq!(Quality::High.next(), Quality::Low);
        assert_eq!(Quality::Low.step(-1), Quality::High);
        for q in [Quality::Low, Quality::Medium, Quality::High] {
            let json = serde_json::to_string(&q).unwrap();
            assert_eq!(json, format!("\"{}\"", q.name()));
            assert_eq!(serde_json::from_str::<Quality>(&json).unwrap(), q);
        }
    }
}
