//! Различия нативной, браузерной (WASM) и Android-сборки: часы, списки ассетов, наличие файлов.
//!
//! В браузере нет файловой системы и `std::time::SystemTime`: списки звуков и моделей
//! вшиваются в сборку, а файлы ассетов загрузчик Bevy берёт по HTTP рядом со страницей.
//! На Android ассеты лежат внутри APK, обычным чтением файла их не достать: списки тоже вшиты.

/// Наносекунды настенных часов для сидов. В браузере — из `Date.now()`.
pub fn clock_nanos() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos() as u64)
    }
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() * 1.0e6) as u64
    }
}

/// Текст списка `assets/<dir>/<file>`.
///
/// Натив читает файл с диска, чтобы правка списка подхватывалась без пересборки.
/// В браузере и на Android списки вшиты в сборку.
pub fn read_list(dir: &str, file: &str) -> Result<String, String> {
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    {
        let path = asset_path(dir, file);
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))
    }
    #[cfg(any(target_arch = "wasm32", target_os = "android"))]
    {
        let text = match (dir, file) {
            ("voice", "lines.json") => include_str!("../assets/voice/lines.json"),
            ("sfx", "sounds.json") => include_str!("../assets/sfx/sounds.json"),
            ("music", "tracks.json") => include_str!("../assets/music/tracks.json"),
            ("models", "models.json") => include_str!("../assets/models/models.json"),
            _ => return Err(format!("{dir}/{file}: список не вшит в сборку")),
        };
        Ok(text.to_owned())
    }
}

/// Есть ли файл `assets/<dir>/<file>`. В браузере и на Android проверить нельзя: считается,
/// что есть, а отсутствующий файл загрузчик отметит как неудачный.
pub fn asset_exists(dir: &str, file: &str) -> bool {
    if file.is_empty() {
        return false;
    }
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    {
        asset_path(dir, file).is_file()
    }
    #[cfg(any(target_arch = "wasm32", target_os = "android"))]
    {
        let _ = dir;
        true
    }
}

/// Ступень качества, которую предлагает страница браузерной версии (`window.spaceGuardQuality`:
/// на телефоне — `low`, вручную — `?quality=`). Android — телефон, ему тоже `low`.
/// Остальной натив подсказок не получает.
pub fn quality_hint() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Reflect::get(&js_sys::global(), &"spaceGuardQuality".into())
            .ok()?
            .as_string()
            .filter(|name| !name.is_empty())
    }
    #[cfg(target_os = "android")]
    {
        Some("low".to_owned())
    }
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    {
        None
    }
}

/// Во сколько раз холст рисует меньше точек, чем у страницы: в браузере на телефоне страница
/// уменьшает холст (`window.spaceGuardCanvasScale`), натив — 1. Размеры в dp делятся на него.
pub fn canvas_scale() -> f32 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Reflect::get(&js_sys::global(), &"spaceGuardCanvasScale".into())
            .ok()
            .and_then(|value| value.as_f64())
            .filter(|scale| *scale >= 1.0)
            .map_or(1.0, |scale| scale as f32)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        1.0
    }
}

/// Отступы от вырезов экрана в dp: слева, сверху, справа, снизу. Страница браузера кладёт их в
/// `window.spaceGuardSafeArea` (`env(safe-area-inset-*)`), на Android — вырез камеры из `WindowInsets`.
pub fn safe_area() -> bevy::math::Vec4 {
    #[cfg(target_os = "android")]
    {
        crate::android::cutout_insets()
    }
    #[cfg(target_arch = "wasm32")]
    {
        use bevy::math::Vec4;
        let read = || -> Option<Vec4> {
            let area = js_sys::Reflect::get(&js_sys::global(), &"spaceGuardSafeArea".into()).ok()?;
            let side = |name: &str| {
                js_sys::Reflect::get(&area, &name.into())
                    .ok()
                    .and_then(|value| value.as_f64())
                    .unwrap_or(0.0) as f32
            };
            Some(Vec4::new(side("left"), side("top"), side("right"), side("bottom")))
        };
        read().unwrap_or(Vec4::ZERO)
    }
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    {
        bevy::math::Vec4::ZERO
    }
}

/// Личный каталог приложения, если его выдаёт система (Android): там лежат рекорды и настройки.
/// На остальных платформах `None`, путь строится по переменным окружения.
pub fn app_data_dir() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "android")]
    {
        bevy::android::ANDROID_APP.get()?.internal_data_path()
    }
    #[cfg(not(target_os = "android"))]
    {
        None
    }
}

/// Сообщает странице браузерной версии, закончена ли загрузка игры (`window.spaceGuardWarm`):
/// экран страницы ждёт её, прежде чем предложить начать. Нативу некому сообщать.
pub fn set_warm(warm: bool) {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = js_sys::Reflect::set(&js_sys::global(), &"spaceGuardWarm".into(), &warm.into());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = warm;
    }
}

/// Игрок нажал «начать» на экране загрузки страницы (`window.spaceGuardEntered`): тогда игра
/// входит в меню. У натива своей страницы нет — всегда да.
pub fn entered() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Reflect::get(&js_sys::global(), &"spaceGuardEntered".into())
            .ok()
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        true
    }
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn asset_path(dir: &str, file: &str) -> std::path::PathBuf {
    bevy::asset::io::file::FileAssetReader::get_base_path()
        .join("assets")
        .join(dir)
        .join(file)
}
