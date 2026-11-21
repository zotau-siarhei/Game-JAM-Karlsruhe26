//! Профиль платформы: все ограничения и облегчения, которые зависят от железа, — плотность кадра,
//! тени, MSAA, обломки, свет, упрощение моделей, прогрев, пулы потоков, ввод по умолчанию.
//!
//! Значения лежат в `assets/platform/profiles.json` (вшит в сборку: профиль нужен до загрузки
//! ассетов, а на Android и в браузере файлов рядом нет). Профиль наследует родителя (`parent`) и
//! перечисляет только отличия; значение со ступенями качества — массив `[low, medium, high]` или
//! одно значение на все ступени. Слои по порядку: база → платформа → класс устройства (iPhone и iPad,
//! браузер на компьютере и на телефоне) → ступень качества → настройки игрока → переопределения.
//!
//! Переопределения для замеров — одно имя на всех платформах, разный транспорт ([`debug_value`]):
//! путь в профиле `render.density` — это `SPACE_GUARD_RENDER_DENSITY=1` на компьютере и iOS,
//! `adb shell setprop debug.spaceguard.render.density 1` на Android и `?render.density=1` в браузере.
//! Профиль целиком — `profile` (`SPACE_GUARD_PROFILE=ios-phone`). Старые имена работают как синонимы
//! ([`ALIASES`]). Обходы ошибок драйверов и систем (Adreno, WebKit, NativeActivity) — не здесь,
//! они остаются условиями платформы в своих модулях.

use std::sync::OnceLock;

use bevy::prelude::*;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::quality::Quality;

/// Файл профилей, вшитый в сборку.
const PROFILES: &str = include_str!("../assets/platform/profiles.json");

/// Класс устройства: от него зависят ввод и вёрстка, а не только мощность.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceClass {
    Desktop,
    Phone,
    Tablet,
}

/// Значение по ступеням качества: low, medium, high.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tiered<T>(pub [T; 3]);

impl<T: Copy> Tiered<T> {
    pub fn at(&self, quality: Quality) -> T {
        self.0[match quality {
            Quality::Low => 0,
            Quality::Medium => 1,
            Quality::High => 2,
        }]
    }
}

impl<'de, T: Deserialize<'de> + Copy> Deserialize<'de> for Tiered<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Form<T> {
            Three([T; 3]),
            One(T),
        }
        Ok(match Form::deserialize(deserializer)? {
            Form::Three(values) => Tiered(values),
            Form::One(value) => Tiered([value; 3]),
        })
    }
}

/// Тени солнца: сторона карты, каскадов, дальность, м.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shadows {
    pub map: usize,
    pub cascades: usize,
    pub far: f32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Render {
    /// Пикселей кадра на точку экрана; `None` — экран как есть (`render_scale.rs`).
    pub density: Tiered<Option<f32>>,
    /// Отсчётов MSAA на высокой ступени; 1 — без MSAA.
    pub msaa_high: u32,
    pub bloom: Tiered<bool>,
    pub shadows: Tiered<Shadows>,
    /// Предел кадров в секунду; `None` — сколько даёт развёртка экрана.
    pub frame_limit: Option<f32>,
    /// Ждать ли развёртку экрана; без неё видно, сколько игра может.
    pub vsync: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Debris {
    /// Сколько уснувших обломков лежит в забеге.
    pub max_resting: usize,
    /// Обломок меньше этого тени не отбрасывает, м.
    pub shadow_min_size: f32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lights {
    /// Пул точечных источников огня.
    pub fire: Tiered<usize>,
    /// Вспышек света от взрывов одновременно.
    pub flashes: usize,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Models {
    /// Упрощать сетки моделей с полем `mobile_detail` (meshoptimizer; в браузере его нет).
    pub simplify: bool,
    /// Множитель доли `mobile_detail`.
    pub detail: f32,
    /// Тень моделей с полем `mobile_shadow` — грубым силуэтом.
    pub shadow_proxies: bool,
    /// Наибольшая сторона текстуры модели после загрузки; `None` — как в файле.
    pub texture_max: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prewarm {
    /// Столько кадров прогрев ждёт перед началом: экран загрузки успевает показаться.
    pub start_frames: u32,
    /// Кадр ждёт, пока соберутся все начатые конвейеры (Adreno вешал GPU при сборке вперемешку).
    pub wait_pipelines: bool,
    /// Собирать конвейеры по одному в рендер-потоке.
    pub sync_compile: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Threads {
    /// Потоков вычислительного пула; `None` — как решит Bevy.
    pub compute: Option<usize>,
    /// Наименьшее и наибольшее число потоков пула асинхронных задач; `None` — как решит Bevy.
    pub async_compute: Option<[usize; 2]>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    /// Касание — устройство ввода с самого запуска.
    pub touch_default: bool,
    /// Есть ли курсор мыши, который прячется при геймпаде.
    pub cursor: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Menu {
    /// Пункт «Выход» в главном меню.
    pub quit: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Audio {
    /// Звук на паузе, пока приложение в фоне.
    pub pause_in_background: bool,
}

/// Итоговый профиль: все поля обязательны, забытое поле — ошибка теста, а не тихое значение.
#[derive(Resource, Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformProfile {
    #[serde(skip)]
    pub name: String,
    /// Действующие переопределения: `путь = значение`.
    #[serde(skip)]
    pub overrides: Vec<String>,
    pub class: DeviceClass,
    pub default_quality: Quality,
    pub render: Render,
    pub debris: Debris,
    pub lights: Lights,
    pub models: Models,
    pub prewarm: Prewarm,
    pub threads: Threads,
    pub input: Input,
    pub menu: Menu,
    pub audio: Audio,
}

impl PlatformProfile {
    /// Плотность кадра на этой ступени.
    pub fn density(&self, quality: Quality) -> Option<f32> {
        self.render.density.at(quality).filter(|d| *d > 0.0)
    }

    /// Тени солнца на этой ступени.
    pub fn shadows(&self, quality: Quality) -> Shadows {
        self.render.shadows.at(quality)
    }

    /// Строка для лога и оверлея: имя, класс, переопределения.
    pub fn describe(&self) -> String {
        let mut text = format!("{} ({:?})", self.name, self.class).to_lowercase();
        if !self.overrides.is_empty() {
            text.push_str(&format!("; переопределено: {}", self.overrides.join(", ")));
        }
        text
    }
}

/// Плагин: профиль в ресурс и строка в лог. Сам профиль собирается раньше, в `main`.
pub struct ProfilePlugin;

impl Plugin for ProfilePlugin {
    fn build(&self, app: &mut App) {
        let profile = get();
        info!("Профиль платформы: {}", profile.describe());
        app.insert_resource(profile.clone());
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(fps) = profile.render.frame_limit.filter(|fps| *fps > 0.0) {
            info!("Предел кадров: {fps} в секунду");
            app.add_systems(Last, limit_frames);
        }
    }
}

/// Простой ограничитель кадров (`render.frame_limit`): в конце кадра поток досыпает до длительности
/// кадра. Нужен там, где развёртка быстрее нужного: iPad с ProMotion даёт 120 Гц, а держать 120
/// кадров игре незачем — греется и садит батарею. iPhone без ключа `CADisableMinimumFrameDurationOnPhone`
/// в `Info.plist` и так не быстрее 60 Гц. В браузере спать нельзя, там кадры задаёт страница.
#[cfg(not(target_arch = "wasm32"))]
fn limit_frames(mut last: Local<Option<bevy::platform::time::Instant>>) {
    let Some(fps) = get().render.frame_limit else {
        return;
    };
    let frame = std::time::Duration::from_secs_f32(1.0 / fps);
    let now = bevy::platform::time::Instant::now();
    if let Some(previous) = *last {
        let elapsed = now.duration_since(previous);
        if elapsed < frame {
            std::thread::sleep(frame - elapsed);
        }
    }
    *last = Some(bevy::platform::time::Instant::now());
}

/// Активный профиль. Собирается при первом обращении — в начале `main`, в главном потоке:
/// на iOS класс устройства спрашивается у UIKit.
pub fn get() -> &'static PlatformProfile {
    static PROFILE: OnceLock<PlatformProfile> = OnceLock::new();
    PROFILE.get_or_init(|| {
        let name = debug_value("profile").unwrap_or_else(|| platform_profile().to_owned());
        let (value, overrides) =
            resolve(&profiles(), &name, &debug_value).unwrap_or_else(|error| {
                panic!("Профиль платформы «{name}»: {error}");
            });
        let mut profile: PlatformProfile = serde_json::from_value(value)
            .unwrap_or_else(|error| panic!("Профиль платформы «{name}»: {error}"));
        profile.name = name;
        profile.overrides = overrides;
        profile
    })
}

/// Слой нагрева: устройство горячее или в режиме энергосбережения (на iOS — `ios::watch_thermal`).
/// Тогда плотность кадра ниже на [`THROTTLE_DENSITY`] (`render_scale.rs`).
static THROTTLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Во сколько раз плотнее кадр, когда устройство горячее: на шаг ниже, как от medium к low.
pub const THROTTLE_DENSITY: f32 = 0.8;

pub fn throttled() -> bool {
    THROTTLED.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub fn set_throttled(on: bool) {
    THROTTLED.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Разобранный файл профилей.
fn profiles() -> Map<String, Value> {
    serde_json::from_str(PROFILES).expect("assets/platform/profiles.json: не JSON-объект")
}

/// Имя профиля этой платформы и класса устройства.
fn platform_profile() -> &'static str {
    #[cfg(target_arch = "wasm32")]
    {
        if crate::platform::is_mobile() {
            "web-mobile"
        } else {
            "web-desktop"
        }
    }
    #[cfg(target_os = "android")]
    {
        "android"
    }
    #[cfg(target_os = "ios")]
    {
        if crate::ios::is_tablet() {
            "ios-tablet"
        } else {
            "ios-phone"
        }
    }
    #[cfg(not(any(target_arch = "wasm32", target_os = "android", target_os = "ios")))]
    {
        // Облегчение для телефона на компьютере — для снимков и сравнения.
        if std::env::var("SPACE_GUARD_MOBILE").is_ok_and(|v| v == "1") {
            "mobile"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else {
            "linux"
        }
    }
}

/// Профиль `name` после наследования и переопределений, и список действующих переопределений.
fn resolve(
    all: &Map<String, Value>,
    name: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<(Value, Vec<String>), String> {
    let mut value = inherited(all, name, 0)?;
    let mut overrides = Vec::new();
    // Старые имена — до путей: путь задаёт точнее и побеждает.
    for (alias, apply) in ALIASES {
        if let Some(text) = lookup(alias) {
            apply(&mut value, text.trim())?;
            overrides.push(format!("{alias} = {}", text.trim()));
        }
    }
    let mut paths = Vec::new();
    leaf_paths(&value, String::new(), &mut paths);
    for path in paths {
        if let Some(text) = lookup(&path) {
            let parsed =
                serde_json::from_str(text.trim()).unwrap_or(Value::String(text.trim().to_owned()));
            set_path(&mut value, &path, parsed);
            overrides.push(format!("{path} = {}", text.trim()));
        }
    }
    Ok((value, overrides))
}

/// Профиль с наследованием: родитель, поверх него — свои поля.
fn inherited(all: &Map<String, Value>, name: &str, depth: usize) -> Result<Value, String> {
    if depth > 8 {
        return Err(format!("цепочка parent длиннее 8 у «{name}»"));
    }
    let own = all
        .get(name)
        .ok_or_else(|| format!("нет профиля «{name}»"))?;
    let mut own = own.clone();
    let parent = own.as_object_mut().and_then(|o| o.remove("parent"));
    let Some(parent) = parent else {
        return Ok(own);
    };
    let parent = parent
        .as_str()
        .ok_or_else(|| format!("parent у «{name}» — не строка"))?;
    let mut value = inherited(all, parent, depth + 1)?;
    merge(&mut value, own);
    Ok(value)
}

/// Глубокое слияние: объекты — по полям, остальное (числа, массивы ступеней) заменяется целиком.
fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(base), Value::Object(over)) => {
            for (key, value) in over {
                match base.get_mut(&key) {
                    Some(slot) => merge(slot, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (slot, value) => *slot = value,
    }
}

/// Пути всех значений профиля: `render.density`, `debris.max_resting`…
fn leaf_paths(value: &Value, prefix: String, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                leaf_paths(value, path, out);
            }
        }
        _ => out.push(prefix),
    }
}

fn set_path(value: &mut Value, path: &str, new: Value) {
    let mut slot = value;
    for key in path.split('.') {
        slot = &mut slot[key];
    }
    *slot = new;
}

/// Синонимы прежних ручек замера: имя → как оно меняет профиль.
type Alias = (&'static str, fn(&mut Value, &str) -> Result<(), String>);
const ALIASES: &[Alias] = &[
    // `density 0` — экран как есть, иначе пикселей на точку на всех ступенях.
    ("density", |value, text| {
        let density: f32 = text
            .parse()
            .map_err(|_| format!("density: не число «{text}»"))?;
        value["render"]["density"] = if density > 0.0 {
            density.into()
        } else {
            Value::Null
        };
        Ok(())
    }),
    // `shadows 1,60` — каскадов и дальность на всех ступенях, карта прежняя.
    ("shadows", |value, text| {
        let (count, far) = text
            .split_once(',')
            .ok_or_else(|| format!("shadows: «{text}», нужно «каскады,метры»"))?;
        let count: usize = count
            .trim()
            .parse()
            .map_err(|_| format!("shadows: «{text}»"))?;
        let far: f32 = far
            .trim()
            .parse()
            .map_err(|_| format!("shadows: «{text}»"))?;
        for tier in value["render"]["shadows"]
            .as_array_mut()
            .into_iter()
            .flatten()
        {
            tier["cascades"] = count.into();
            tier["far"] = far.into();
        }
        Ok(())
    }),
    ("detail", |value, text| {
        value["models"]["detail"] = text
            .parse::<f32>()
            .map_err(|_| format!("detail: «{text}»"))?
            .into();
        Ok(())
    }),
    ("compute", |value, text| {
        value["threads"]["compute"] = text
            .parse::<usize>()
            .map_err(|_| format!("compute: «{text}»"))?
            .into();
        Ok(())
    }),
    ("sync", |value, text| {
        value["prewarm"]["sync_compile"] = (text == "1").into();
        Ok(())
    }),
    ("novsync", |value, text| {
        value["render"]["vsync"] = (text != "1").into();
        Ok(())
    }),
];

/// Значение отладочного переопределения `key` (`render.density`, `off`, `quality`) или `None`.
/// Компьютер и iOS — переменная `SPACE_GUARD_<KEY>` (точки и дефисы → `_`, верхний регистр);
/// Android — свойство `debug.spaceguard.<key>`, затем та же переменная (её задаёт
/// `debug.spaceguard.env`); браузер — параметр адреса `?<key>=`.
pub fn debug_value(key: &str) -> Option<String> {
    #[cfg(target_os = "android")]
    if let Some(value) = std::ffi::CString::new(format!("debug.spaceguard.{key}"))
        .ok()
        .and_then(|name| crate::android::system_property(&name))
    {
        return Some(value);
    }
    #[cfg(target_arch = "wasm32")]
    {
        url_parameter(key)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var(env_name(key)).ok().filter(|v| !v.is_empty())
    }
}

/// Имя переменной окружения для ключа: `render.density` → `SPACE_GUARD_RENDER_DENSITY`.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn env_name(key: &str) -> String {
    format!(
        "SPACE_GUARD_{}",
        key.replace(['.', '-'], "_").to_uppercase()
    )
}

/// Параметр адреса страницы: `?render.density=1&off=grass`.
#[cfg(target_arch = "wasm32")]
fn url_parameter(key: &str) -> Option<String> {
    let location = js_sys::Reflect::get(&js_sys::global(), &"location".into()).ok()?;
    let search = js_sys::Reflect::get(&location, &"search".into())
        .ok()?
        .as_string()?;
    search.trim_start_matches('?').split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        let value = js_sys::decode_uri_component(value).ok()?.as_string()?;
        (name == key && !value.is_empty()).then_some(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [&str; 11] = [
        "base",
        "desktop",
        "macos",
        "linux",
        "windows",
        "web-desktop",
        "web-mobile",
        "mobile",
        "android",
        "ios-phone",
        "ios-tablet",
    ];

    fn build(name: &str, overrides: &[(&str, &str)]) -> PlatformProfile {
        let lookup = |key: &str| {
            overrides
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        };
        let (value, list) = resolve(&profiles(), name, &lookup).unwrap();
        let mut profile: PlatformProfile = serde_json::from_value(value).unwrap();
        profile.name = name.to_owned();
        profile.overrides = list;
        profile
    }

    #[test]
    fn каждый_профиль_собирается_со_всеми_полями() {
        for name in ALL {
            build(name, &[]);
        }
    }

    /// Перенос на профили не меняет компьютер, браузер и Android: числа — как были в коде.
    #[test]
    fn профили_повторяют_прежние_числа() {
        let desktop_shadows = [(512, 1, 40.0), (1024, 2, 70.0), (2048, 3, 110.0)];
        let phone_shadows = [(512, 1, 40.0), (1024, 1, 60.0), (2048, 2, 70.0)];
        let shadows = |p: &PlatformProfile| {
            [Quality::Low, Quality::Medium, Quality::High].map(|q| {
                let s = p.shadows(q);
                (s.map, s.cascades, s.far)
            })
        };
        let tiers = [Quality::Low, Quality::Medium, Quality::High];
        for name in ["macos", "linux", "windows", "web-desktop"] {
            let p = build(name, &[]);
            assert_eq!(p.class, DeviceClass::Desktop, "{name}");
            assert_eq!(p.default_quality, Quality::Medium, "{name}");
            assert_eq!(tiers.map(|q| p.density(q)), [None; 3], "{name}");
            assert_eq!(p.render.msaa_high, 4, "{name}");
            assert_eq!(p.render.bloom.0, [false, true, true], "{name}");
            assert_eq!(shadows(&p), desktop_shadows, "{name}");
            assert_eq!(
                (p.debris.max_resting, p.debris.shadow_min_size),
                (1500, 0.8),
                "{name}"
            );
            assert_eq!(
                (p.lights.fire.0, p.lights.flashes),
                ([4, 6, 8], 6),
                "{name}"
            );
            assert!(
                !p.models.simplify && !p.models.shadow_proxies && p.models.texture_max.is_none(),
                "{name}"
            );
            assert_eq!(
                (p.prewarm.start_frames, p.prewarm.wait_pipelines),
                (0, false),
                "{name}"
            );
            assert!(!p.input.touch_default && p.input.cursor, "{name}");
            assert_eq!(p.menu.quit, name != "web-desktop", "{name}");
        }
        let web = build("web-mobile", &[]);
        assert_eq!(web.default_quality, Quality::Low);
        assert_eq!(tiers.map(|q| web.density(q)), [None; 3]);
        assert_eq!((web.render.msaa_high, shadows(&web)), (4, desktop_shadows));
        assert_eq!(
            (web.debris.max_resting, web.debris.shadow_min_size),
            (600, 1.2)
        );
        assert_eq!((web.lights.fire.0, web.lights.flashes), ([2, 3, 4], 3));
        assert_eq!(web.models.texture_max, Some(1024));
        assert!(!web.input.touch_default && !web.menu.quit);

        let android = build("android", &[]);
        assert_eq!(android.default_quality, Quality::Low);
        assert_eq!(
            tiers.map(|q| android.density(q)),
            [Some(1.25), Some(1.5), Some(2.0)]
        );
        assert_eq!(
            (android.render.msaa_high, shadows(&android)),
            (1, phone_shadows)
        );
        assert_eq!(
            (android.debris.max_resting, android.debris.shadow_min_size),
            (600, 1.2)
        );
        assert_eq!(
            (android.lights.fire.0, android.lights.flashes),
            ([2, 3, 4], 3)
        );
        assert!(android.models.simplify && android.models.shadow_proxies);
        assert_eq!(android.models.texture_max, Some(1024));
        assert_eq!(
            (android.prewarm.start_frames, android.prewarm.wait_pipelines),
            (3, true)
        );
        assert_eq!(
            (android.threads.compute, android.threads.async_compute),
            (Some(4), Some([2, 6]))
        );
        assert!(android.input.touch_default && !android.input.cursor && android.menu.quit);
        assert!(android.audio.pause_in_background);
    }

    #[test]
    fn ipad_наследует_iphone_и_меняет_своё() {
        let phone = build("ios-phone", &[]);
        let tablet = build("ios-tablet", &[]);
        assert_eq!(phone.class, DeviceClass::Phone);
        assert_eq!(tablet.class, DeviceClass::Tablet);
        assert_eq!(phone.density(Quality::Medium), Some(1.5));
        assert_eq!(phone.shadows(Quality::Medium).cascades, 1);
        assert_eq!(phone.render.msaa_high, 4);
        assert!(!phone.menu.quit && phone.input.touch_default && phone.audio.pause_in_background);
        assert_eq!(tablet.debris.max_resting, 800);
        assert_eq!(tablet.render.frame_limit, Some(60.0));
        // Не заданное у iPad берётся у iPhone.
        assert_eq!(tablet.menu.quit, phone.menu.quit);
        assert_eq!(tablet.models, phone.models);
    }

    #[test]
    fn переопределения_путём_и_старыми_именами() {
        let p = build(
            "android",
            &[("render.density", "1.0"), ("debris.max_resting", "100")],
        );
        assert_eq!(p.density(Quality::High), Some(1.0));
        assert_eq!(p.debris.max_resting, 100);
        assert_eq!(p.overrides.len(), 2);
        let p = build(
            "android",
            &[
                ("density", "0"),
                ("shadows", "2,80"),
                ("detail", "0.5"),
                ("novsync", "1"),
            ],
        );
        assert_eq!(p.density(Quality::Medium), None);
        assert_eq!(
            (
                p.shadows(Quality::Medium).cascades,
                p.shadows(Quality::Medium).far
            ),
            (2, 80.0)
        );
        assert_eq!(p.shadows(Quality::Medium).map, 1024);
        assert_eq!(p.models.detail, 0.5);
        assert!(!p.render.vsync);
        // Путь точнее синонима и побеждает его.
        let p = build(
            "macos",
            &[("density", "2"), ("render.density", "[1, 1.5, 2]")],
        );
        assert_eq!(p.density(Quality::Low), Some(1.0));
    }

    #[test]
    fn имена_переменных() {
        assert_eq!(env_name("render.density"), "SPACE_GUARD_RENDER_DENSITY");
        assert_eq!(env_name("off"), "SPACE_GUARD_OFF");
        assert_eq!(env_name("detail"), "SPACE_GUARD_DETAIL");
    }
}
