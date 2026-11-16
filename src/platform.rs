//! Различия нативной и браузерной (WASM) сборки: часы, списки ассетов и наличие файлов.
//!
//! В браузере нет файловой системы и `std::time::SystemTime`: списки звуков и моделей
//! вшиваются в сборку, а файлы ассетов загрузчик Bevy берёт по HTTP рядом со страницей.

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
/// В браузере списки вшиты в сборку.
pub fn read_list(dir: &str, file: &str) -> Result<String, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = asset_path(dir, file);
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let text = match (dir, file) {
            ("voice", "lines.json") => include_str!("../assets/voice/lines.json"),
            ("sfx", "sounds.json") => include_str!("../assets/sfx/sounds.json"),
            ("music", "tracks.json") => include_str!("../assets/music/tracks.json"),
            ("models", "models.json") => include_str!("../assets/models/models.json"),
            _ => return Err(format!("{dir}/{file}: список не вшит в браузерную сборку")),
        };
        Ok(text.to_owned())
    }
}

/// Есть ли файл `assets/<dir>/<file>`. В браузере проверить нельзя: считается, что есть,
/// а отсутствующий файл загрузчик отметит как неудачный.
pub fn asset_exists(dir: &str, file: &str) -> bool {
    if file.is_empty() {
        return false;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        asset_path(dir, file).is_file()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = dir;
        true
    }
}

/// Ступень качества, которую предлагает страница браузерной версии (`window.spaceGuardQuality`:
/// на телефоне — `low`, вручную — `?quality=`). Натив подсказок не получает.
pub fn quality_hint() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Reflect::get(&js_sys::global(), &"spaceGuardQuality".into())
            .ok()?
            .as_string()
            .filter(|name| !name.is_empty())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn asset_path(dir: &str, file: &str) -> std::path::PathBuf {
    bevy::asset::io::file::FileAssetReader::get_base_path()
        .join("assets")
        .join(dir)
        .join(file)
}
