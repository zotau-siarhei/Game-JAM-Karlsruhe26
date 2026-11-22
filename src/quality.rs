//! Ступени качества картинки: свечение, тени и сглаживание. Переключаются клавишей Q на паузе
//! и в меню настроек, выбор сохраняется вместе с остальными настройками (`settings.rs`).
//!
//! | Ступень | Свечение | Тени | Сглаживание |
//! |---|---|---|---|
//! | Низкая | нет | нет | нет |
//! | Средняя | есть | карта 1024, два каскада до 70 м | нет |
//! | Высокая | есть | карта 2048, три каскада до 110 м | MSAA 4× |
//!
//! Числа таблицы — для компьютера; у каждой платформы они свои, в профиле (`profile.rs`): на телефонах
//! средняя ступень — один каскад до 60 м, высокая — два каскада до 70 м, на Android без MSAA
//! (с ним Adreno 810 теряет устройство). Плотность кадра по ступени — `src/render_scale.rs`.
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
        // Для замеров ступень сильнее файла настроек: `SPACE_GUARD_QUALITY=high`,
        // `adb shell setprop debug.spaceguard.quality high`, `?quality=high` (`profile::debug_value`).
        let forced: Option<Quality> = crate::profile::debug_value("quality")
            .and_then(|name| serde_json::from_value(serde_json::Value::String(name.trim().to_owned())).ok());
        let quality = match forced {
            Some(forced) => {
                // Меню берёт ступень из настроек, поэтому она ставится и туда, но в файл не пишется.
                app.insert_resource(crate::settings::ForcedQuality { forced, stored: quality });
                app.world_mut().resource_mut::<Settings>().quality = forced;
                forced
            }
            None => quality,
        };
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
    if crate::perf::off("hdr") {
        return;
    }
    commands
        .entity(*camera)
        .insert((bevy::camera::Hdr, Tonemapping::TonyMcMapface));
}

/// Наибольшее число выборок MSAA не больше `wanted` из тех, что есть у Bevy (1, 2, 4, 8) и
/// поддерживает видеокарта.
fn supported_samples(wanted: u32, supported: impl Fn(u32) -> bool) -> u32 {
    [8, 4, 2].into_iter().find(|&n| n <= wanted && supported(n)).unwrap_or(1)
}

/// Q на паузе меняет ступень по кругу.
fn switch_quality(keys: Res<ButtonInput<KeyCode>>, mut quality: ResMut<Quality>) {
    if keys.just_pressed(KeyCode::KeyQ) {
        *quality = quality.next();
    }
}

/// Новая ступень — в настройки: они сохраняют её в файл. Ступень из отладочного переопределения
/// (`quality`) в файл не попадает (`settings::ForcedQuality`): после замера игрок остаётся на своей.
fn remember_quality(mut commands: Commands, quality: Res<Quality>, mut settings: ResMut<Settings>) {
    // Игрок сам сменил ступень — дальше она его, даже если совпадёт с переопределением.
    if !quality.is_added() {
        commands.remove_resource::<crate::settings::ForcedQuality>();
    }
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
    adapter: Option<Res<bevy::render::renderer::RenderAdapter>>,
) {
    let q = *quality;
    let profile = crate::profile::get();
    let bloom = profile.render.bloom.at(q) && !crate::perf::off("bloom");
    // MSAA только на высокой ступени и сколько разрешает профиль (на Android — нет), но не больше,
    // чем умеет видеокарта: 8 выборок у iPhone до A17 нет, и Bevy упал бы при создании цели.
    let wanted = if q != Quality::High || crate::perf::off("msaa") { 1 } else { profile.render.msaa_high };
    let samples = match &adapter {
        Some(adapter) => {
            use bevy::render::render_resource::TextureFormat;
            let formats = [TextureFormat::Rgba16Float, TextureFormat::Bgra8UnormSrgb, TextureFormat::Depth32Float]
                .map(|format| adapter.get_texture_format_features(format).flags);
            supported_samples(wanted, |n| formats.iter().all(|flags| flags.sample_count_supported(n)))
        }
        None => supported_samples(wanted, |_| true),
    };
    if samples != wanted {
        warn!("MSAA {wanted}: видеокарта не поддерживает, взято {samples}");
    }
    let msaa = match samples {
        2 => Msaa::Sample2,
        4 => Msaa::Sample4,
        8 => Msaa::Sample8,
        _ => Msaa::Off,
    };
    for camera in &camera {
        let mut c = commands.entity(camera);
        if bloom {
            c.insert(Bloom::NATURAL);
        } else {
            c.remove::<Bloom>();
        }
        c.insert(msaa);
    }
    let shadows = profile.shadows(q);
    let (size, cascades, far) = (shadows.map, shadows.cascades, shadows.far);
    info!(
        "Ступень {}: свечение {bloom}, MSAA {msaa:?}, тени {} (карта {size}, каскадов {cascades} до {far} м)",
        q.name(),
        if q == Quality::Low || crate::perf::off("shadows") { "нет" } else { "есть" },
    );
    commands.insert_resource(DirectionalLightShadowMap { size });
    for (entity, mut sun) in &mut suns {
        sun.shadow_maps_enabled = q != Quality::Low && !crate::perf::off("shadows");
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
    fn msaa_не_больше_возможностей_видеокарты() {
        let ios = |n: u32| matches!(n, 1 | 2 | 4);
        assert_eq!(supported_samples(8, ios), 4);
        assert_eq!(supported_samples(4, ios), 4);
        assert_eq!(supported_samples(16, |_| true), 8);
        assert_eq!(supported_samples(3, ios), 2);
        assert_eq!(supported_samples(4, |n| n == 1 || n == 4), 4);
        assert_eq!(supported_samples(2, |n| n == 1 || n == 4), 1);
        assert_eq!(supported_samples(1, ios), 1);
    }

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
