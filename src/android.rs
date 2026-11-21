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

/// Системное свойство Android (`adb shell setprop <имя> <значение>`); `None` — не задано.
/// Свойства `debug.*` ставятся без root: ими замеры переключаются без пересборки.
pub fn system_property(name: &std::ffi::CStr) -> Option<String> {
    unsafe extern "C" {
        fn __system_property_get(name: *const std::ffi::c_char, value: *mut std::ffi::c_char) -> i32;
    }
    let mut buffer = [0 as std::ffi::c_char; 92];
    // SAFETY: имя — строка с нулём на конце, буфер не короче PROP_VALUE_MAX (92).
    let length = unsafe { __system_property_get(name.as_ptr(), buffer.as_mut_ptr()) };
    if length <= 0 {
        return None;
    }
    // SAFETY: функция записала строку с нулём на конце в буфер.
    let text = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) }.to_str().ok()?;
    (!text.is_empty()).then(|| text.to_owned())
}

/// Отступы от выреза камеры в dp: слева, сверху, справа, снизу. С target SDK 35+ окно рисуется и
/// под вырезом (edge-to-edge принудительно), поэтому сенсорные кнопки его обходят сами.
/// Спрашивает систему не чаще раза в секунду: телефон переворачивают, и вырез переходит на другой бок.
pub fn cutout_insets() -> Vec4 {
    use std::sync::Mutex;
    static CACHE: Mutex<Option<(bevy::platform::time::Instant, Vec4)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let now = bevy::platform::time::Instant::now();
    if let Some((at, insets)) = *cache
        && now.duration_since(at).as_secs_f32() < 1.0
    {
        return insets;
    }
    let insets = read_cutout().unwrap_or(Vec4::ZERO);
    *cache = Some((now, insets));
    insets
}

/// `Activity.getWindow().getDecorView().getRootWindowInsets().getDisplayCutout()`, в dp.
fn read_cutout() -> Option<Vec4> {
    use jni::JavaVM;
    use jni::objects::JObject;
    let app = bevy::android::ANDROID_APP.get()?;
    // SAFETY: указатели на JavaVM и активность выдаёт система, они действуют, пока жив процесс.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
    let mut env = vm.attach_current_thread_permanently().ok()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let found = (|| -> jni::errors::Result<Option<Vec4>> {
        let window = env.call_method(&activity, "getWindow", "()Landroid/view/Window;", &[])?.l()?;
        let decor = env.call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?.l()?;
        let insets = env
            .call_method(&decor, "getRootWindowInsets", "()Landroid/view/WindowInsets;", &[])?
            .l()?;
        if insets.is_null() {
            return Ok(None);
        }
        let cutout = env.call_method(&insets, "getDisplayCutout", "()Landroid/view/DisplayCutout;", &[])?.l()?;
        if cutout.is_null() {
            return Ok(Some(Vec4::ZERO));
        }
        let resources = env.call_method(&activity, "getResources", "()Landroid/content/res/Resources;", &[])?.l()?;
        let metrics = env
            .call_method(&resources, "getDisplayMetrics", "()Landroid/util/DisplayMetrics;", &[])?
            .l()?;
        let density = env.get_field(&metrics, "density", "F")?.f()?.max(0.5);
        let mut side = |name: &str| env.call_method(&cutout, name, "()I", &[]).and_then(|v| v.i());
        let px = Vec4::new(
            side("getSafeInsetLeft")? as f32,
            side("getSafeInsetTop")? as f32,
            side("getSafeInsetRight")? as f32,
            side("getSafeInsetBottom")? as f32,
        );
        Ok(Some(px / density))
    })();
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    found.ok().flatten()
}
