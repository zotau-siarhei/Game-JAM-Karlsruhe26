//! Своя копия крейта подменяет его через `[patch.crates-io]` только при той же версии. Если обновить
//! зависимость, cargo лишь предупредит, что заплатка не используется, и исправления молча пропадут.
//! Этот тест падает, чтобы при обновлении заплатку пересмотрели.

/// Версия пакета в `Cargo.lock` и признак, что он взят из реестра, а не из своей копии.
fn locked(name: &str) -> Vec<(String, bool)> {
    let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock")).expect("Cargo.lock");
    lock.split("[[package]]")
        .filter(|block| block.lines().any(|line| line.trim() == format!("name = \"{name}\"")))
        .map(|block| {
            let version = block
                .lines()
                .find_map(|line| line.trim().strip_prefix("version = "))
                .unwrap_or_default()
                .trim_matches('"')
                .to_owned();
            (version, block.contains("source = \"registry+"))
        })
        .collect()
}

#[test]
fn копия_wgpu_hal_в_ходу() {
    let found = locked("wgpu-hal");
    assert_eq!(
        found,
        vec![("30.0.2".to_owned(), false)],
        "wgpu-hal в Cargo.lock не из vendor/wgpu-hal: wgpu обновился. Проверь, есть ли в новой версии исправления \
         из vendor/wgpu-hal/PATCH.md (#10230, #10124, #10264). Если есть — удали копию и строку wgpu-hal в \
         [patch.crates-io] (Cargo.toml и android/Cargo.toml), если нет — перенеси их в копию новой версии. \
         Затем поправь этот тест."
    );
}

#[test]
fn копия_bevy_audio_в_ходу() {
    let found = locked("bevy_audio");
    assert_eq!(
        found,
        vec![("0.20.1".to_owned(), false)],
        "bevy_audio в Cargo.lock не из vendor/bevy_audio: Bevy обновился. Перенеси правки из \
         vendor/bevy_audio/PATCH.md в копию новой версии или удали копию, если они уже в Bevy. Затем поправь этот тест."
    );
}
