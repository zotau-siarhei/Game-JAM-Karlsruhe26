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
        app.add_systems(PreUpdate, (read_view, watch_thermal));
    }
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
        msg_send![&*device, userInterfaceIdiom]
    };
    idiom == PAD
}

/// Слой нагрева профиля: раз в 2 с спрашивает `NSProcessInfo` о нагреве (`thermalState`) и режиме
/// энергосбережения. При нагреве «serious» и выше или в энергосбережении плотность кадра на шаг ниже
/// (`profile::set_throttled`), назад — когда остыл. iPhone 15 Pro Max в долгой нагрузке держит
/// 60–76 % пиковой мощности видеокарты: лучше заранее рисовать меньше, чем терять кадры.
fn watch_thermal(time: Res<Time<Real>>, mut next: Local<f32>) {
    /// `NSProcessInfoThermalStateSerious`.
    const SERIOUS: isize = 2;
    let now = time.elapsed_secs();
    if now < *next {
        return;
    }
    *next = now + 2.0;
    // SAFETY: `NSProcessInfo.processInfo` есть всегда, оба свойства только читаются.
    let (thermal, low_power): (isize, bool) = unsafe {
        let info: *mut AnyObject = msg_send![objc2::class!(NSProcessInfo), processInfo];
        (msg_send![&*info, thermalState], msg_send![&*info, isLowPowerModeEnabled])
    };
    let throttle = thermal >= SERIOUS || low_power;
    if throttle != crate::profile::throttled() {
        info!("iOS: нагрев {thermal}, энергосбережение {low_power} → кадр {}", if throttle { "реже" } else { "как в профиле" });
        crate::profile::set_throttled(throttle);
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
    let view = handle.ui_view.as_ptr().cast::<AnyObject>();
    // SAFETY: указатель на UIView окна выдал winit, окно живо, пока есть его сущность; вызов идёт
    // из главного потока (`NonSendMarker`).
    let (insets, bounds): (UIEdgeInsets, CGRect) =
        unsafe { (msg_send![&*view, safeAreaInsets], msg_send![&*view, bounds]) };
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
        // SAFETY: те же вид и поток, что выше; `layer` и `mainScreen` у UIKit есть всегда.
        let (view_scale, layer_scale, screen_scale, native_scale): (f64, f64, f64, f64) = unsafe {
            let layer: *mut AnyObject = msg_send![&*view, layer];
            let screen: *mut AnyObject = msg_send![objc2::class!(UIScreen), mainScreen];
            (
                msg_send![&*view, contentScaleFactor],
                msg_send![&*layer, contentsScale],
                msg_send![&*screen, scale],
                msg_send![&*screen, nativeScale],
            )
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
