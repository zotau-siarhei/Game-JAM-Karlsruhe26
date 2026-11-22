//! Особенности нативной сборки под Android: обход ошибок драйвера Adreno, уровень устройства,
//! реакция на нагрев, память и Game Mode, частота экрана.
//!
//! Сборка и упаковка APK — `scripts/build-android.sh`, крейт библиотеки — `android/Cargo.toml`.

use std::borrow::Cow;
use std::ffi::c_void;

use bevy::prelude::*;
use bevy::render::batching::gpu_preprocessing::{GpuPreprocessingMode, GpuPreprocessingSupport};
use bevy::render::error_handler::{RenderError, RenderErrorHandler, RenderErrorPolicy};
use bevy::render::renderer::RenderDevice;
use bevy::render::{Render, RenderApp, RenderStartup, init_gpu_resource};
use bevy::shader::{Shader, Source};


/// Начало выборки, на которой падает компилятор шейдеров Adreno.
const COMPARE_CALL: &str = "textureSampleCompareLevel(";
/// Первый аргумент выборки из массива кубических карт теней точечных источников.
const POINT_SHADOW_ARGUMENT: &str = "view_bindings::point_shadow_textures,";

pub struct AndroidPlugin;

impl Plugin for AndroidPlugin {
    fn build(&self, app: &mut App) {
        std::sync::LazyLock::force(&START);
        restart_on_panic();
        let device = device();
        info!(
            "Android: GPU «{}», памяти {:.1} ГБ, low_ram {}, SoC {} → уровень {:?} (опрос {:.0} мс)",
            device.gpu,
            device.memory as f64 / (1u64 << 30) as f64,
            device.low_ram,
            device.soc,
            device.tier(),
            device.probe_ms,
        );
        app.add_systems(Last, drop_point_shadow_sampling)
            .init_resource::<Load>()
            .add_systems(PreUpdate, (watch_thermal, watch_memory, watch_game_mode, apply_load).chain())
            .add_systems(Last, request_frame_rate)
            .insert_resource(RenderErrorHandler(restart_on_render_error));
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.add_systems(
                RenderStartup,
                disable_gpu_preprocessing.after(init_gpu_resource::<GpuPreprocessingSupport>),
            );
            // Проверка перезапуска: `adb shell setprop debug.spaceguard.lose <секунд>` — через
            // столько секунд после запуска видеокарта «теряется».
            if let Some(seconds) = system_property(c"debug.spaceguard.lose").and_then(|v| v.parse::<f32>().ok()) {
                render_app.add_systems(Render, move |device: Res<RenderDevice>, mut done: Local<bool>| {
                    if !*done && START.elapsed().as_secs_f32() > seconds {
                        *done = true;
                        warn!("Проверка: видеокарта теряется нарочно");
                        device.wgpu_device().destroy();
                    }
                });
            }
        }
    }
}

/// Что известно об устройстве до запуска Bevy: по этому выбирается профиль (`profile::android_tier`).
pub struct Device {
    /// Имя GPU у адаптера Vulkan: «Adreno (TM) 810», «Mali-G715», пусто — не узнали.
    pub gpu: String,
    /// `MemTotal` из `/proc/meminfo`, байт; 0 — не прочитали.
    pub memory: u64,
    /// Система считает устройство слабым (`ro.config.low_ram`).
    pub low_ram: bool,
    /// Производитель и модель SoC (`ro.soc.*`) — только для лога.
    pub soc: String,
    /// Сколько заняло определение, мс.
    pub probe_ms: f32,
}

impl Device {
    pub fn tier(&self) -> crate::profile::DeviceTier {
        crate::profile::android_tier(&self.gpu, self.memory, self.low_ram)
    }
}

/// Устройство; определяется один раз, при сборке профиля в начале `main`.
pub fn device() -> &'static Device {
    static DEVICE: std::sync::OnceLock<Device> = std::sync::OnceLock::new();
    DEVICE.get_or_init(|| {
        let start = bevy::platform::time::Instant::now();
        let gpu = vulkan_gpu_name().unwrap_or_default();
        let memory = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|text| {
                let line = text.lines().find(|line| line.starts_with("MemTotal:"))?;
                line.split_whitespace().nth(1)?.parse::<u64>().ok()
            })
            .map_or(0, |kib| kib << 10);
        let low_ram = system_property(c"ro.config.low_ram").is_some_and(|value| value == "true");
        let soc = [c"ro.soc.manufacturer", c"ro.soc.model"]
            .into_iter()
            .filter_map(system_property)
            .collect::<Vec<_>>()
            .join(" ");
        Device { gpu, memory, low_ram, soc, probe_ms: start.elapsed().as_secs_f32() * 1000.0 }
    })
}

/// Имя первого не программного GPU у временного экземпляра Vulkan: Bevy своё создаёт позже, а профиль
/// нужен раньше — по нему настраиваются пулы потоков и окно.
fn vulkan_gpu_name() -> Option<String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::empty(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapters = bevy::tasks::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN));
    adapters
        .iter()
        .map(wgpu::Adapter::get_info)
        .find(|info| info.device_type != wgpu::DeviceType::Cpu)
        .map(|info| info.name)
}

/// Функции `libandroid.so` новее minSdk 29: ищутся при первом обращении, нет на телефоне — `None`.
struct Ndk {
    /// `AThermalManager*` (API 30); 0 — нет.
    thermal_manager: usize,
    /// `AThermal_getCurrentThermalStatus` (API 30).
    thermal_status: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
    /// `AThermal_getThermalHeadroom` (API 31).
    thermal_headroom: Option<unsafe extern "C" fn(*mut c_void, i32) -> f32>,
    /// `ANativeWindow_setFrameRate` (API 30).
    set_frame_rate: Option<unsafe extern "C" fn(*mut c_void, f32, i8) -> i32>,
}


fn ndk() -> &'static Ndk {
    static NDK: std::sync::OnceLock<Ndk> = std::sync::OnceLock::new();
    NDK.get_or_init(|| {
        unsafe extern "C" {
            fn dlopen(filename: *const std::ffi::c_char, flag: i32) -> *mut c_void;
            fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
        }
        /// `RTLD_NOW` в bionic.
        const RTLD_NOW: i32 = 2;
        // SAFETY: имя — строка с нулём на конце; библиотека системная и не выгружается.
        let library = unsafe { dlopen(c"libandroid.so".as_ptr(), RTLD_NOW) };
        let symbol = |name: &std::ffi::CStr| -> *mut c_void {
            if library.is_null() {
                return std::ptr::null_mut();
            }
            // SAFETY: библиотека открыта, имя — строка с нулём на конце.
            unsafe { dlsym(library, name.as_ptr()) }
        };
        let acquire = symbol(c"AThermal_acquireManager");
        // SAFETY: у каждой функции сигнатура из заголовков NDK (`android/thermal.h`,
        // `android/native_window.h`); указатель не нулевой.
        unsafe {
            let thermal_manager = if acquire.is_null() {
                0
            } else {
                std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> *mut c_void>(acquire)() as usize
            };
            let status = symbol(c"AThermal_getCurrentThermalStatus");
            let headroom = symbol(c"AThermal_getThermalHeadroom");
            let frame_rate = symbol(c"ANativeWindow_setFrameRate");
            Ndk {
                thermal_manager,
                thermal_status: (!status.is_null()).then(|| std::mem::transmute(status)),
                thermal_headroom: (!headroom.is_null()).then(|| std::mem::transmute(headroom)),
                set_frame_rate: (!frame_rate.is_null()).then(|| std::mem::transmute(frame_rate)),
            }
        }
    })
}

/// Нагрузка, которую игра снимает с телефона сама: нагрев и режим Game Mode «экономия батареи».
/// Источники объединяются — хватает одного (`apply_load`).
#[derive(Resource, Default, PartialEq)]
struct Load {
    /// По запасу до троттлинга (`watch_thermal`); `None` — ещё не спрашивали или запаса нет.
    headroom: Option<crate::profile::Heat>,
    /// По статусу нагрева — только где запаса нет (Android 10 или HAL без прогноза, запас всегда NaN):
    /// гистерезис у статуса свой, в HAL.
    status: Option<crate::profile::Heat>,
    /// Game Mode «экономия батареи» (`watch_game_mode`).
    battery: bool,
}

impl Load {
    fn heat(&self) -> crate::profile::Heat {
        use crate::profile::Heat;
        let battery = if self.battery { Heat::Overheated } else { Heat::Cool };
        [self.headroom, self.status].into_iter().flatten().fold(battery, Heat::max)
    }
}

/// Раз в 10 с, как советует Google, — запас до троттлинга (ADPF Thermal): от 0,95 кадр реже, от 1,0 —
/// не больше 30 кадров (`profile::heat_from_headroom`), назад — с запасом на остывание. Запас 1,0 по
/// определению — порог статуса SEVERE, поэтому статус нагрева нужен только там, где запаса нет
/// (Android 11, система не умеет его считать): MODERATE — кадр реже, SEVERE и выше — 30 кадров.
/// NaN — система не успела посчитать (слишком частый опрос, сбой HAL): остаётся прежний уровень.
fn watch_thermal(time: Res<Time<Real>>, mut next: Local<f32>, mut load: ResMut<Load>) {
    use crate::profile::Heat;
    /// `ATHERMAL_STATUS_MODERATE`, `ATHERMAL_STATUS_SEVERE`.
    const MODERATE: i32 = 2;
    const SEVERE: i32 = 3;
    let now = time.elapsed_secs();
    if now < *next {
        return;
    }
    *next = now + 10.0;
    let ndk = ndk();
    if ndk.thermal_manager == 0 {
        return;
    }
    let manager = ndk.thermal_manager as *mut c_void;
    // SAFETY: менеджер получен у системы и не освобождается; сигнатура — из `android/thermal.h`.
    let raw = ndk.thermal_headroom.map(|get_headroom| unsafe { get_headroom(manager, 0) });
    let (headroom, status) = if let Some(raw) = raw.filter(|raw| raw.is_finite()) {
        let was = load.headroom.unwrap_or(Heat::Cool);
        (Some(crate::profile::heat_from_headroom(raw, was)), None)
    } else if load.headroom.is_some() {
        // Запас уже приходил числом: случайный NaN держит прежний уровень, а не сбрасывает его.
        return;
    } else if let Some(get_status) = ndk.thermal_status {
        // Запаса нет совсем: функция есть с Android 11, но без прогноза в HAL устройства
        // всегда отдаёт NaN. Тогда — статус нагрева.
        // SAFETY: то же.
        let raw = unsafe { get_status(manager) };
        let heat = match raw {
            s if s >= SEVERE => Heat::Overheated,
            s if s >= MODERATE => Heat::Throttled,
            _ => Heat::Cool,
        };
        (None, Some(heat))
    } else {
        return;
    };
    if (load.headroom, load.status) != (headroom, status) {
        info!("Android: нагрев по запасу {headroom:?}, по статусу {status:?}");
        load.headroom = headroom;
        load.status = status;
    }
}

/// Раз в 10 с — `ActivityManager.getMemoryInfo`: с Android 14 `onLowMemory` не приходит, а
/// `onTrimMemory` — только при уходе в фон. Система говорит «мало памяти» — обломков вдвое меньше;
/// назад — когда свободно в полтора раза больше порога системы.
fn watch_memory(time: Res<Time<Real>>, mut next: Local<f32>, mut logged: Local<bool>) {
    use jni::objects::JValue;
    let now = time.elapsed_secs();
    if now < *next {
        return;
    }
    *next = now + 10.0;
    let Some((low, available, threshold)) = with_activity(|env, activity| {
        let name = env.new_string("activity")?;
        let manager = env
            .call_method(activity, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&name)])?
            .l()?;
        let info = env.new_object("android/app/ActivityManager$MemoryInfo", "()V", &[])?;
        env.call_method(&manager, "getMemoryInfo", "(Landroid/app/ActivityManager$MemoryInfo;)V", &[JValue::Object(&info)])?;
        Ok((
            env.get_field(&info, "lowMemory", "Z")?.z()?,
            env.get_field(&info, "availMem", "J")?.j()?,
            env.get_field(&info, "threshold", "J")?.j()?,
        ))
    }) else {
        return;
    };
    if !*logged {
        *logged = true;
        info!("Android: свободно {} МБ, порог системы {} МБ", available >> 20, threshold >> 20);
    }
    let was = crate::profile::low_memory();
    let now_low = low || (was && available < threshold * 3 / 2);
    if now_low != was {
        warn!(
            "Android: свободно {} МБ, порог системы {} МБ → обломков {}",
            available >> 20,
            threshold >> 20,
            if now_low { "вдвое меньше" } else { "как в профиле" }
        );
        crate::profile::set_low_memory(now_low);
    }
}

/// Режим Game Mode (Android 12+) — при запуске и при каждом возврате в игру: игрок меняет его в
/// панели игр, пока игра в фоне. «Экономия батареи» — не больше 30 кадров и кадр на шаг реже.
fn watch_game_mode(
    mut lifecycle: MessageReader<bevy::window::AppLifecycle>,
    mut checked: Local<bool>,
    mut load: ResMut<Load>,
) {
    use jni::objects::JValue;
    /// `GameManager.GAME_MODE_BATTERY`.
    const BATTERY: i32 = 3;
    // Bevy 0.20 шлёт при возврате `Running`, а не `WillResume` (см. `sound::pause_audio_in_background`).
    let resumed = lifecycle.read().any(|event| {
        matches!(event, bevy::window::AppLifecycle::WillResume | bevy::window::AppLifecycle::Running)
    });
    let first = !*checked;
    if !first && !resumed {
        return;
    }
    *checked = true;
    let Some(mode) = with_activity(|env, activity| {
        let name = env.new_string("game")?;
        let manager = env
            .call_method(activity, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&name)])?
            .l()?;
        // До Android 12 службы игр нет.
        if manager.is_null() {
            return Ok(None);
        }
        env.call_method(&manager, "getGameMode", "()I", &[])?.i().map(Some)
    })
    .flatten() else {
        return;
    };
    let battery = mode == BATTERY;
    if battery != load.battery || first {
        info!("Android: Game Mode {mode}{}", if battery { " — экономия батареи" } else { "" });
    }
    load.battery = battery;
}

/// Сводит источники нагрузки в флаги профиля: кадр реже (`throttled`) и не больше 30 кадров
/// (`overheated`).
fn apply_load(load: Res<Load>) {
    use crate::profile::Heat;
    if !load.is_changed() {
        return;
    }
    let heat = load.heat();
    if (heat >= Heat::Throttled) != crate::profile::throttled() || (heat == Heat::Overheated) != crate::profile::overheated() {
        info!(
            "Android: кадр {}, кадров {}",
            if heat >= Heat::Throttled { "реже" } else { "как в профиле" },
            if heat == Heat::Overheated { format!("не больше {}", crate::profile::OVERHEAT_FPS) } else { "как в профиле".into() }
        );
    }
    crate::profile::set_throttled(heat >= Heat::Throttled);
    crate::profile::set_overheated(heat == Heat::Overheated);
}

/// Частота экрана по пределу кадров: на экране 90/120 Гц игра с пределом 60 иначе рисует неровно,
/// а экран зря тратит батарею. Окно пересоздаётся при каждом возврате в игру, а предел меняется при
/// нагреве, поэтому раз в секунду проверяется, что системе сказано то, что нужно. При уходе в фон
/// запомненное сбрасывается: новое окно может получить адрес старого. Неудачный вызов повторяется.
fn request_frame_rate(
    time: Res<Time<Real>>,
    mut lifecycle: MessageReader<bevy::window::AppLifecycle>,
    bench: Option<Res<crate::perf::Bench>>,
    mut next: Local<f32>,
    mut applied: Local<Option<(usize, Option<f32>)>>,
) {
    /// `ANATIVEWINDOW_FRAME_RATE_COMPATIBILITY_DEFAULT`.
    const COMPATIBILITY_DEFAULT: i8 = 0;
    if lifecycle.read().any(|event| matches!(event, bevy::window::AppLifecycle::WillSuspend | bevy::window::AppLifecycle::Suspended)) {
        // Окно в этом кадре ещё старое (android-activity отпускает его после `TermWindow`):
        // частоту ему не просить, иначе новое окно по тому же адресу сочлось бы уже настроенным.
        *applied = None;
        *next = 0.0;
        return;
    }
    let now = time.elapsed_secs();
    if now < *next {
        return;
    }
    *next = now + 1.0;
    let Some(set_frame_rate) = ndk().set_frame_rate else {
        return;
    };
    let Some(window) = bevy::android::ANDROID_APP.get().and_then(|app| app.native_window()) else {
        // Игра в фоне: окна нет, после возврата частота просится заново.
        *applied = None;
        return;
    };
    let pointer = window.ptr().as_ptr();
    // `--bench` меряет без предела кадров — и без просьбы к экрану.
    let wanted = if bench.is_some_and(|bench| bench.on) { None } else { crate::profile::current_frame_limit() };
    if *applied == Some((pointer as usize, wanted)) {
        return;
    }
    // 0 — частота на усмотрение системы.
    // SAFETY: окно захвачено `native_window` и живо, пока жив `window`.
    let result = unsafe { set_frame_rate(pointer.cast(), wanted.unwrap_or(0.0), COMPATIBILITY_DEFAULT) };
    info!("Android: частота экрана {} (ответ {result})", wanted.map_or("системная".into(), |fps| format!("{fps} Гц")));
    *applied = (result == 0).then_some((pointer as usize, wanted));
}

/// Время запуска процесса для `debug.spaceguard.lose`.
static START: std::sync::LazyLock<bevy::platform::time::Instant> =
    std::sync::LazyLock::new(bevy::platform::time::Instant::now);

/// Потеря видеокарты (DeviceLost): драйвер Adreno изредка сбрасывает устройство, чаще всего при
/// сборке шейдеров в загрузке. Тогда wgpu паникует в пуле задач, а если успеет Bevy — закрывает игру
/// ошибкой рендера; игра выглядит упавшей. Вместо этого она открывается заново: новая активность
/// в живом процессе завершает его (`main`), и система поднимает её в новом процессе с новым устройством.
fn restart_on_render_error(error: &RenderError, main_world: &mut World, _: &mut World) -> RenderErrorPolicy {
    if restart_once() {
        error!("Ошибка рендера {:?}: игра открывается заново", error.ty);
    } else {
        error!("Ошибка рендера {:?}: игра закрывается", error.ty);
    }
    main_world.write_message(AppExit::error());
    RenderErrorPolicy::StopRendering
}

/// Паника тоже перезапускает игру: при потере видеокарты wgpu паникует раньше обработчика ошибок рендера.
fn restart_on_panic() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        if restart_once() {
            error!("Паника: игра открывается заново");
        }
    }));
}

/// Дополнение запуска, по которому новый процесс знает, что он — перезапуск.
const RESTARTED_EXTRA: &str = "space_guard.restarted";
/// Перезапуск, упавший раньше этого срока, игру уже не перезапускает: так не будет круга запусков.
const RESTART_GRACE_SECONDS: f32 = 60.0;

/// Перезапускает игру один раз за процесс. `false` — не стал или не вышло.
fn restart_once() -> bool {
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if DONE.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return true;
    }
    if START.elapsed().as_secs_f32() < RESTART_GRACE_SECONDS && with_activity(|env, activity| {
        let intent = env.call_method(activity, "getIntent", "()Landroid/content/Intent;", &[])?.l()?;
        let name = env.new_string(RESTARTED_EXTRA)?;
        env.call_method(&intent, "getBooleanExtra", "(Ljava/lang/String;Z)Z", &[jni::objects::JValue::Object(&name), false.into()])?
            .z()
    }) != Some(false)
    {
        return false;
    }
    relaunch()
}

/// Вызов Java в подключённом к JVM потоке; `None` — JNI недоступен или вызов бросил исключение.
/// Потоки игры подключаются навсегда, а у такого потока нет Java-кадра, который снял бы локальные
/// ссылки: без своего кадра они копились бы при каждом вызове до переполнения таблицы.
pub(crate) fn with_env<T>(call: impl FnOnce(&mut jni::JNIEnv) -> jni::errors::Result<T>) -> Option<T> {
    let app = bevy::android::ANDROID_APP.get()?;
    // SAFETY: указатель на JavaVM выдаёт система, он действует, пока жив процесс.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
    let mut env = vm.attach_current_thread_permanently().ok()?;
    let result = env.with_local_frame(16, |env| call(env));
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
        return None;
    }
    result.ok()
}

/// Вызов Java с активностью игры (см. [`with_env`]).
fn with_activity<T>(
    call: impl FnOnce(&mut jni::JNIEnv, &jni::objects::JObject) -> jni::errors::Result<T>,
) -> Option<T> {
    let app = bevy::android::ANDROID_APP.get()?;
    // SAFETY: указатель на активность выдаёт система, она живёт, пока жив процесс.
    let activity = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr().cast()) };
    with_env(|env| call(env, &activity))
}

/// Запускает свою активность заново в новой задаче, прежнюю закрывает. `false` — не вышло.
fn relaunch() -> bool {
    use jni::objects::JValue;
    with_activity(|env, activity| {
        let manager = env
            .call_method(activity, "getPackageManager", "()Landroid/content/pm/PackageManager;", &[])?
            .l()?;
        let package = env.call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?.l()?;
        let intent = env
            .call_method(
                &manager,
                "getLaunchIntentForPackage",
                "(Ljava/lang/String;)Landroid/content/Intent;",
                &[JValue::Object(&package)],
            )?
            .l()?;
        if intent.is_null() {
            return Ok(false);
        }
        // FLAG_ACTIVITY_NEW_TASK | FLAG_ACTIVITY_CLEAR_TASK.
        env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(0x1000_0000 | 0x0000_8000)])?;
        let name = env.new_string(RESTARTED_EXTRA)?;
        env.call_method(
            &intent,
            "putExtra",
            "(Ljava/lang/String;Z)Landroid/content/Intent;",
            &[JValue::Object(&name), true.into()],
        )?;
        env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
        Ok(true)
    })
    .unwrap_or(false)
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

/// Аргументы запуска и переменные окружения для проверок на телефоне, где их не передать:
/// `adb shell setprop debug.spaceguard.args "--bench=60"` и
/// `adb shell setprop debug.spaceguard.env "SPACE_GUARD_AUTOSTART=1;SPACE_GUARD_SCENE=fire"`.
/// Возвращает аргументы, переменные ставит сразу.
pub fn debug_launch() -> Vec<String> {
    if let Some(env) = system_property(c"debug.spaceguard.env") {
        for pair in env.split(';') {
            if let Some((key, value)) = pair.split_once('=') {
                // SAFETY: вызывается в начале `main`, до того как игра запустила свои потоки
                // и стала читать окружение.
                unsafe { std::env::set_var(key.trim(), value.trim()) };
                info!("Проверка: {}={}", key.trim(), value.trim());
            }
        }
    }
    let mut args: Vec<String> = std::env::args().collect();
    if let Some(extra) = system_property(c"debug.spaceguard.args") {
        args.extend(extra.split_whitespace().map(str::to_owned));
    }
    args
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
/// Не вышло спросить — остаётся прежнее значение.
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
    let insets = read_cutout().or(cache.map(|(_, insets)| insets)).unwrap_or(Vec4::ZERO);
    *cache = Some((now, insets));
    insets
}

/// `Display.getCutout()` в dp. Вырез берётся у экрана, а не у окна (`View.getRootWindowInsets`):
/// `Display` можно спрашивать из любого потока, а View — только из потока интерфейса, а игра
/// зовёт это из своих потоков. `DisplayInfo` повёрнут вместе с экраном, так что стороны совпадают
/// с окном игры на весь экран.
fn read_cutout() -> Option<Vec4> {
    with_activity(|env, activity| {
        let manager = env.call_method(activity, "getWindowManager", "()Landroid/view/WindowManager;", &[])?.l()?;
        let display = env.call_method(&manager, "getDefaultDisplay", "()Landroid/view/Display;", &[])?.l()?;
        if display.is_null() {
            return Ok(None);
        }
        let cutout = env.call_method(&display, "getCutout", "()Landroid/view/DisplayCutout;", &[])?.l()?;
        if cutout.is_null() {
            return Ok(Some(Vec4::ZERO));
        }
        let resources = env.call_method(activity, "getResources", "()Landroid/content/res/Resources;", &[])?.l()?;
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
    })
    .flatten()
}
