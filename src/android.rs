//! Особенности нативной сборки под Android: обход ошибок драйвера Adreno и звук в фоне.
//!
//! Сборка и упаковка APK — `scripts/build-android.sh`, крейт библиотеки — `android/Cargo.toml`.

use std::borrow::Cow;

use bevy::audio::{AudioSinkPlayback, SpatialAudioSink};
use bevy::prelude::*;
use bevy::render::batching::gpu_preprocessing::{GpuPreprocessingMode, GpuPreprocessingSupport};
use bevy::render::{RenderApp, RenderStartup, init_gpu_resource};
use bevy::shader::{Shader, Source};
use bevy::window::AppLifecycle;

use crate::frame::GameSet;

/// Начало выборки, на которой падает компилятор шейдеров Adreno.
const COMPARE_CALL: &str = "textureSampleCompareLevel(";
/// Первый аргумент выборки из массива кубических карт теней точечных источников.
const POINT_SHADOW_ARGUMENT: &str = "view_bindings::point_shadow_textures,";

pub struct AndroidPlugin;

impl Plugin for AndroidPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Last, drop_point_shadow_sampling)
            .add_systems(Update, pause_audio_in_background.in_set(GameSet::Presentation));
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.add_systems(
                RenderStartup,
                disable_gpu_preprocessing.after(init_gpu_resource::<GpuPreprocessingSupport>),
            );
        }
    }
}

/// Выключает предобработку и отсечение мешей на видеокарте, как Bevy делает сам на Adreno до 730:
/// с этой настройкой игра проверена на Adreno 810. Вторая половина настройки —
/// `use_gpu_instance_buffer_builder` в `main.rs`.
fn disable_gpu_preprocessing(mut support: ResMut<GpuPreprocessingSupport>) {
    support.max_supported_mode = GpuPreprocessingMode::None;
    info!("Android: предобработка мешей на GPU выключена");
}

/// Заменяет выборку тени точечного источника на 1.0, как страница браузерной версии: компилятор
/// шейдеров Adreno падает на ней при сборке первого же PBR-конвейера. Тени у точечных источников
/// в игре выключены, выборка и так дала бы 1.0. Система стоит в `Last`: шейдер меняется в том же
/// кадре, в котором загрузился, до передачи в рендер.
fn drop_point_shadow_sampling(mut done: Local<bool>, mut shaders: ResMut<Assets<Shader>>) {
    if *done {
        return;
    }
    let found = shaders.iter().find_map(|(id, shader)| {
        let (Source::Wgsl(text) | Source::Wesl(text)) = &shader.source else {
            return None;
        };
        without_point_shadow_sampling(text).map(|patched| (id, patched))
    });
    let Some((id, patched)) = found else {
        return;
    };
    if let Some(mut shader) = shaders.get_mut(id)
        && let Source::Wgsl(text) | Source::Wesl(text) = &mut shader.source
    {
        *text = Cow::Owned(patched);
        info!("Android: выборка тени точечного источника заменена на 1.0");
    }
    *done = true;
}

/// Текст шейдера без выборки теней точечных источников. `None` — такой выборки в нём нет.
fn without_point_shadow_sampling(text: &str) -> Option<String> {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    let mut changed = false;
    while let Some(start) = rest.find(COMPARE_CALL) {
        let arguments = &rest[start + COMPARE_CALL.len()..];
        let mut depth = 1;
        let length = arguments.find(|symbol| {
            match symbol {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            depth == 0
        })?;
        let call_end = start + COMPARE_CALL.len() + length + 1;
        if arguments.trim_start().starts_with(POINT_SHADOW_ARGUMENT) {
            result.push_str(&rest[..start]);
            result.push_str("1.0");
            changed = true;
        } else {
            result.push_str(&rest[..call_end]);
        }
        rest = &rest[call_end..];
    }
    result.push_str(rest);
    changed.then_some(result)
}

/// Система Android не глушит приложение в фоне: уходя с экрана, игра ставит звук на паузу сама.
/// Звуки мира на паузе игры снова замрут сами (`sound::pause_world_sounds`).
fn pause_audio_in_background(
    mut lifecycle: MessageReader<AppLifecycle>,
    sinks: Query<&AudioSink>,
    spatial_sinks: Query<&SpatialAudioSink>,
) {
    for event in lifecycle.read() {
        match event {
            AppLifecycle::WillSuspend => {
                sinks.iter().for_each(|sink| sink.pause());
                spatial_sinks.iter().for_each(|sink| sink.pause());
            }
            AppLifecycle::WillResume => {
                sinks.iter().for_each(|sink| sink.play());
                spatial_sinks.iter().for_each(|sink| sink.play());
            }
            _ => {}
        }
    }
}
