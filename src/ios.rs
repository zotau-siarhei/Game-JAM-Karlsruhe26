//! Особенности iOS: каталог данных в песочнице, размер окна и безопасная область экрана.
//!
//! winit 0.30 на iOS не сообщает Bevy размер экрана: окно остаётся 1280×720 точек по умолчанию,
//! кадр рисуется больше экрана и обрезается. Размер берётся у вида UIKit. Безопасную область
//! (вырез, Dynamic Island, полоска «домой») тоже знает только UIKit, а спрашивать UIKit можно лишь
//! из главного потока. Поэтому обоих читает система с `NonSendMarker`, а остальной код берёт
//! последнюю область из [`safe_area`].

use std::path::PathBuf;
use std::sync::Mutex;

use bevy::ecs::system::NonSendMarker;
use bevy::prelude::*;
use bevy::window::{PrimaryWindow, RawHandleWrapper};
use objc2::encode::{Encode, Encoding};
use objc2::msg_send;
use objc2::runtime::AnyObject;
use raw_window_handle::RawWindowHandle;

// Среда C++ для meshoptimizer (`src/lod.rs`): сборочный скрипт крейта `meshopt` подключает libc++
// только для целей `*-darwin`, а цель iOS — `aarch64-apple-ios`, и без этого не находятся
// `operator new` и `__gxx_personality_v0`.
#[link(name = "c++")]
unsafe extern "C" {}

/// Плагин iOS: размер окна и безопасная область по виду UIKit.
pub struct IosPlugin;

impl Plugin for IosPlugin {
    fn build(&self, app: &mut App) {
        let d = device();
        info!(
            "iOS: GPU Apple{}, памяти {:.1} ГБ, до предела процесса {:.1} ГБ, {} → уровень {:?}",
            d.apple_family,
            d.physical as f64 / GIB,
            d.available as f64 / GIB,
            if d.tablet { "iPad" } else { "iPhone" },
            d.tier(),
        );
        app.add_systems(PreUpdate, (read_view, watch_thermal, log_memory));
    }
}

const GIB: f64 = (1u64 << 30) as f64;

#[link(name = "Metal", kind = "framework")]
unsafe extern "C" {
    fn MTLCreateSystemDefaultDevice() -> *mut AnyObject;
}

unsafe extern "C" {
    /// libSystem, iOS 13+: байт до предела памяти процесса (в симуляторе — 0).
    fn os_proc_available_memory() -> usize;
}

/// Возможности устройства, по которым выбирается профиль (`profile::device_tier`).
#[derive(Debug, Clone, Copy)]
pub struct Device {
    /// Наибольшее семейство GPU Metal `Apple<N>`, которое поддерживает видеокарта (0 — нет Metal).
    pub apple_family: u32,
    /// Объём памяти, байт.
    pub physical: u64,
    /// Сколько памяти до предела процесса при запуске, байт (0 — неизвестно).
    pub available: u64,
    pub tablet: bool,
}

impl Device {
    pub fn tier(&self) -> crate::profile::DeviceTier {
        crate::profile::device_tier(self.apple_family, self.physical, self.available, self.tablet)
    }
}

/// Возможности устройства, один раз за запуск. Спрашивать из главного потока до цикла событий:
/// профиль платформы собирается в начале `main`, раньше адаптера wgpu.
pub fn device() -> Device {
    static DEVICE: std::sync::OnceLock<Device> = std::sync::OnceLock::new();
    *DEVICE.get_or_init(|| {
        /// `MTLGPUFamilyApple1`; `Apple<N>` = 1000 + N.
        const APPLE: isize = 1000;
        // SAFETY: функции Metal и Foundation без побочных эффектов; объекты живут всё время процесса.
        // Каждый ответ может оказаться nil — тогда ссылки нет (`as_ref`), а не разыменование нуля.
        unsafe {
            let metal = MTLCreateSystemDefaultDevice().as_ref();
            // Семейства накопительные: берётся самое новое, которое устройство поддерживает.
            let apple_family = metal.map_or(0, |metal| {
                (1..=16).rev().find(|n| msg_send![metal, supportsFamily: APPLE + *n as isize]).unwrap_or(0)
            });
            let info: *mut AnyObject = msg_send![objc2::class!(NSProcessInfo), processInfo];
            let physical: u64 = info.as_ref().map_or(0, |info| msg_send![info, physicalMemory]);
            Device {
                apple_family,
                physical,
                available: os_proc_available_memory() as u64,
                tablet: is_tablet(),
            }
        }
    })
}

/// Последние отступы безопасной области в точках (dp): слева, сверху, справа, снизу.
static SAFE_AREA: Mutex<Vec4> = Mutex::new(Vec4::ZERO);

/// Отступы безопасной области в dp: слева, сверху, справа, снизу.
pub fn safe_area() -> Vec4 {
    *SAFE_AREA.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `Library/Application Support` песочницы приложения: туда пишутся рекорды и настройки.
pub fn data_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library")
            .join("Application Support"),
    )
}

/// iPad, а не iPhone: `UIDevice.currentDevice.userInterfaceIdiom == UIUserInterfaceIdiomPad`.
/// Спрашивать из главного потока: профиль платформы собирается в начале `main`.
pub fn is_tablet() -> bool {
    /// `UIUserInterfaceIdiomPad`.
    const PAD: isize = 1;
    // SAFETY: `UIDevice.currentDevice` есть всегда; вызов из главного потока до запуска цикла событий.
    let idiom: isize = unsafe {
        let device: *mut AnyObject = msg_send![objc2::class!(UIDevice), currentDevice];
        device.as_ref().map_or(0, |device| msg_send![device, userInterfaceIdiom])
    };
    idiom == PAD
}

/// Слой нагрева профиля: раз в 2 с спрашивает `NSProcessInfo` о нагреве (`thermalState`) и режиме
/// энергосбережения. При нагреве «serious» и выше или в энергосбережении плотность кадра на шаг ниже
/// (`profile::set_throttled`), назад — когда остыл. iPhone 15 Pro Max в долгой нагрузке держит
/// 60–76 % пиковой мощности видеокарты: лучше заранее рисовать меньше, чем терять кадры.
fn watch_thermal(time: Res<Time<Real>>, mut next: Local<f32>) {
    /// `NSProcessInfoThermalStateSerious`, `…Critical`.
    const SERIOUS: isize = 2;
    const CRITICAL: isize = 3;
    let now = time.elapsed_secs();
    if now < *next {
        return;
    }
    *next = now + 2.0;
    // SAFETY: `NSProcessInfo.processInfo` есть всегда, оба свойства только читаются.
    let Some((thermal, low_power)): Option<(isize, bool)> = (unsafe {
        let info: *mut AnyObject = msg_send![objc2::class!(NSProcessInfo), processInfo];
        info.as_ref().map(|info| (msg_send![info, thermalState], msg_send![info, isLowPowerModeEnabled]))
    }) else {
        return;
    };
    let throttle = thermal >= SERIOUS || low_power;
    if throttle != crate::profile::throttled() {
        info!("iOS: нагрев {thermal}, энергосбережение {low_power} → кадр {}", if throttle { "реже" } else { "как в профиле" });
        crate::profile::set_throttled(throttle);
    }
    // Критический нагрев — ещё и не больше 30 кадров: система вот-вот начнёт душить сама.
    let overheated = thermal >= CRITICAL;
    if overheated != crate::profile::overheated() {
        info!(
            "iOS: нагрев {thermal} → кадров {}",
            if overheated { format!("не больше {}", crate::profile::OVERHEAT_FPS) } else { "как в профиле".into() }
        );
        crate::profile::set_overheated(overheated);
    }
}

/// Лог игры ещё и в файл `Library/Application Support/space_guard.log`: с iPhone системный лог
/// без прав администратора не достать, а файл песочницы забирает
/// `xcrun devicectl device copy from --domain-type appDataContainer --domain-identifier
/// io.github.zotausiarhei.spaceguard --source "Library/Application Support/space_guard.log"`.
/// Файл начинается заново при каждом запуске.
pub fn file_log_layer(_app: &mut App) -> Option<bevy::log::BoxedLayer> {
    use bevy::log::tracing_subscriber::{Layer, fmt};
    let dir = data_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let file = std::fs::File::create(dir.join("space_guard.log")).ok()?;
    Some(
        fmt::layer()
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file))
            .boxed(),
    )
}

/// Меньше этого до предела процесса — «мало памяти»: обломков вдвое меньше (`profile::low_memory`).
const LOW_MEMORY: usize = 300 << 20;
/// Больше этого — памяти снова хватает (запас, чтобы флаг не мигал).
const ENOUGH_MEMORY: usize = 500 << 20;

/// Раз в 10 с — сколько памяти ещё может взять игра, пока система её не закрыла (jetsam).
/// Когда её мало, игра держит меньше обломков.
fn log_memory(time: Res<Time<Real>>, mut next: Local<f32>) {
    let now = time.elapsed_secs();
    if now < *next {
        return;
    }
    *next = now + 10.0;
    // SAFETY: функция без аргументов, только читает счётчик процесса.
    let available = unsafe { os_proc_available_memory() };
    // В симуляторе предела нет, функция отдаёт 0.
    if available == 0 {
        return;
    }
    info!("iOS: памяти до предела {} МБ", available / (1024 * 1024));
    let low = crate::profile::low_memory();
    if !low && available < LOW_MEMORY || low && available > ENOUGH_MEMORY {
        warn!(
            "iOS: памяти до предела {} МБ → обломков {}",
            available >> 20,
            if low { "как в профиле" } else { "вдвое меньше" }
        );
        crate::profile::set_low_memory(!low);
    }
}

/// `UIEdgeInsets` из UIKit.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct UIEdgeInsets {
    top: f64,
    left: f64,
    bottom: f64,
    right: f64,
}

// SAFETY: раскладка совпадает со структурой UIKit: четыре CGFloat (f64 на 64-битных устройствах).
unsafe impl Encode for UIEdgeInsets {
    const ENCODING: Encoding = Encoding::Struct(
        "UIEdgeInsets",
        &[f64::ENCODING, f64::ENCODING, f64::ENCODING, f64::ENCODING],
    );
}

/// `CGRect` из CoreGraphics: начало и размер, CGFloat — f64.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CGRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

// SAFETY: раскладка совпадает с `CGRect { CGPoint origin; CGSize size; }` на 64-битных устройствах.
unsafe impl Encode for CGRect {
    const ENCODING: Encoding = Encoding::Struct(
        "CGRect",
        &[
            Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]),
            Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]),
        ],
    );
}

/// Размер окна и `safeAreaInsets` у вида окна игры. Только главный поток.
fn read_view(
    mut window: Query<(&RawHandleWrapper, &mut Window), With<PrimaryWindow>>,
    mut last: Local<Option<Vec4>>,
    _main_thread: NonSendMarker,
) {
    let Ok((handle, mut bevy_window)) = window.single_mut() else {
        return;
    };
    let RawWindowHandle::UiKit(handle) = handle.get_window_handle() else {
        return;
    };
    // SAFETY: указатель на UIView окна выдал winit (`NonNull`, не nil), окно живо, пока есть его
    // сущность; вызов идёт из главного потока (`NonSendMarker`).
    let view: &AnyObject = unsafe { handle.ui_view.cast::<AnyObject>().as_ref() };
    let (insets, bounds): (UIEdgeInsets, CGRect) =
        unsafe { (msg_send![view, safeAreaInsets], msg_send![view, bounds]) };
    let size = Vec2::new(bounds.width as f32, bounds.height as f32);
    if size.min_element() > 0.0 && (bevy_window.resolution.size() - size).abs().max_element() > 0.5 {
        info!("iOS: окно {} → {size} точек", bevy_window.resolution.size());
        bevy_window.resolution.set(size.x, size.y);
    }
    let area = Vec4::new(
        insets.left as f32,
        insets.top as f32,
        insets.right as f32,
        insets.bottom as f32,
    );
    if *last != Some(area) {
        // Масштабы вида, его слоя и экрана: по ним видно, совпадает ли кадр с экраном.
        // SAFETY: те же вид и поток, что выше; nil у слоя или экрана — масштаб 0 в логе.
        let (view_scale, layer_scale, screen_scale, native_scale): (f64, f64, f64, f64) = unsafe {
            let layer: *mut AnyObject = msg_send![view, layer];
            let screen: *mut AnyObject = msg_send![objc2::class!(UIScreen), mainScreen];
            let layer_scale = layer.as_ref().map_or(0.0, |layer| msg_send![layer, contentsScale]);
            let (scale, native) = screen
                .as_ref()
                .map_or((0.0, 0.0), |screen| (msg_send![screen, scale], msg_send![screen, nativeScale]));
            (msg_send![view, contentScaleFactor], layer_scale, scale, native)
        };
        info!(
            "iOS: безопасная область {area}; окно {} точек, {} пикселей, масштаб {}; вид {view_scale}, \
             слой {layer_scale}, экран {screen_scale} ({native_scale})",
            bevy_window.resolution.size(),
            bevy_window.resolution.physical_size(),
            bevy_window.resolution.scale_factor(),
        );
        *last = Some(area);
        *SAFE_AREA.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = area;
    }
}
