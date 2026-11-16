//! Световая панель DualShock 4 и DualSense на Linux: через светодиоды драйвера ядра в
//! `/sys/class/leds`.
//!
//! Драйвер `hid-playstation` (DualSense, DualShock 4 с ядра 6.2) даёт многоцветный светодиод
//! `inputN:rgb:indicator` с файлами `multi_intensity` и `brightness`, старый `hid-sony` —
//! три светодиода `…:054C:…:red`, `:green`, `:blue`. По умолчанию писать в них может только root;
//! без прав панель не меняется, а в лог один раз пишется, какое правило udev добавить.

// На macOS модуль собирается только для тестов.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::fs;
use std::path::{Path, PathBuf};

use bevy::prelude::*;

use super::PadLight;

/// Где ядро показывает светодиоды.
const LEDS: &str = "/sys/class/leds";
/// Как часто перечитывать список светодиодов (геймпад могли подключить), с.
const RESCAN: f32 = 3.0;
/// USB vendor id Sony в верхнем регистре, как его пишет ядро.
const SONY: &str = "054C";

pub(super) fn build(app: &mut App) {
    app.init_resource::<SysLeds>()
        .add_systems(Update, apply.after(crate::frame::GameSet::Presentation));
}

/// Светодиод панели одного геймпада.
#[derive(Debug, Clone, PartialEq)]
enum Led {
    /// `hid-playstation`: один многоцветный светодиод.
    Multi(PathBuf),
    /// `hid-sony`: три светодиода по цветам.
    Rgb { red: PathBuf, green: PathBuf, blue: PathBuf },
}

/// Светодиоды панелей геймпадов PlayStation в каталоге `root` (обычно `/sys/class/leds`).
fn scan(root: &Path) -> Vec<Led> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    let mut leds = Vec::new();
    for name in &names {
        let dir = root.join(name);
        if name.ends_with(":rgb:indicator") && dir.join("multi_intensity").exists() {
            // Светодиод «indicator» бывает и не у геймпада: проверяем производителя устройства.
            let uevent = fs::read_to_string(dir.join("device/uevent")).unwrap_or_default();
            if uevent.to_uppercase().contains(SONY) {
                leds.push(Led::Multi(dir));
            }
        } else if let Some(base) = name.strip_suffix(":red") {
            if !base.to_uppercase().contains(SONY) {
                continue;
            }
            let green = root.join(format!("{base}:green"));
            let blue = root.join(format!("{base}:blue"));
            if green.exists() && blue.exists() {
                leds.push(Led::Rgb { red: dir, green, blue });
            }
        }
    }
    leds
}

/// Доля цвета 0..1 — в яркость 0..255.
fn level(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Записывает цвет в светодиод.
fn write(led: &Led, color: Vec3) -> std::io::Result<()> {
    match led {
        Led::Multi(dir) => {
            fs::write(
                dir.join("multi_intensity"),
                format!("{} {} {}", level(color.x), level(color.y), level(color.z)),
            )?;
            fs::write(dir.join("brightness"), "255")
        }
        Led::Rgb { red, green, blue } => {
            fs::write(red.join("brightness"), level(color.x).to_string())?;
            fs::write(green.join("brightness"), level(color.y).to_string())?;
            fs::write(blue.join("brightness"), level(color.z).to_string())
        }
    }
}

#[derive(Resource, Default)]
struct SysLeds {
    leds: Vec<Led>,
    /// Сколько осталось до перечитывания списка, с.
    rescan: f32,
    /// Последний записанный цвет: пишем только изменения.
    written: Option<Vec3>,
    /// Отказ уже в логе.
    warned: bool,
}

fn apply(light: Res<PadLight>, time: Res<Time<Real>>, mut leds: ResMut<SysLeds>) {
    leds.rescan -= time.delta_secs();
    if leds.rescan <= 0.0 {
        leds.rescan = RESCAN;
        let found = scan(Path::new(LEDS));
        if found != leds.leds {
            if !found.is_empty() {
                info!("Световая панель геймпада: светодиодов {}", found.len());
            }
            leds.leds = found;
            leds.written = None;
        }
    }
    if leds.leds.is_empty() || leds.written.is_some_and(|c| c.distance(light.0) < 0.02) {
        return;
    }
    let mut failed = None;
    for led in &leds.leds {
        if let Err(error) = write(led, light.0) {
            failed = Some(error);
        }
    }
    match failed {
        None => leds.written = Some(light.0),
        Some(error) => {
            // Не долбим запись каждый кадр: следующая попытка — с перечитыванием списка.
            leds.written = Some(light.0);
            if !leds.warned {
                leds.warned = true;
                warn!(
                    "Световая панель геймпада: нет записи в {LEDS} ({error}). Права даёт правило udev, \
                     например /etc/udev/rules.d/70-space-guard-lightbar.rules:\n\
                     ACTION==\"add\", SUBSYSTEM==\"leds\", KERNEL==\"input*:rgb:indicator|*054C*\", \
                     RUN+=\"/bin/sh -c 'chmod a+w /sys%p/brightness /sys%p/multi_intensity 2>/dev/null; true'\""
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn находит_панели_обоих_драйверов_и_пишет_цвет() {
        let root = std::env::temp_dir().join(format!("space-guard-leds-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // hid-playstation: многоцветный светодиод, устройство Sony.
        let multi = root.join("input12:rgb:indicator");
        fs::create_dir_all(multi.join("device")).unwrap();
        fs::write(multi.join("multi_intensity"), "0 0 0").unwrap();
        fs::write(multi.join("brightness"), "0").unwrap();
        fs::write(multi.join("device/uevent"), "HID_ID=0005:0000054C:000009CC\n").unwrap();
        // Чужой многоцветный светодиод — не наш.
        let other = root.join("input3:rgb:indicator");
        fs::create_dir_all(other.join("device")).unwrap();
        fs::write(other.join("multi_intensity"), "0 0 0").unwrap();
        fs::write(other.join("device/uevent"), "HID_ID=0003:0000046D:0000C52B\n").unwrap();
        // hid-sony: три светодиода.
        for color in ["red", "green", "blue"] {
            let dir = root.join(format!("0005:054C:09CC.0003:{color}"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("brightness"), "0").unwrap();
        }

        let leds = scan(&root);
        assert_eq!(leds.len(), 2, "{leds:?}");
        for led in &leds {
            write(led, Vec3::new(1.0, 0.5, 0.0)).unwrap();
        }
        assert_eq!(fs::read_to_string(multi.join("multi_intensity")).unwrap(), "255 128 0");
        assert_eq!(fs::read_to_string(multi.join("brightness")).unwrap(), "255");
        assert_eq!(fs::read_to_string(root.join("0005:054C:09CC.0003:green/brightness")).unwrap(), "128");
        let _ = fs::remove_dir_all(&root);
    }
}
