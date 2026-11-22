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
    /// Отклонённые переопределения с причиной: опечатка в значении не роняет игру.
    #[serde(skip)]
    pub rejected: Vec<String>,
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
    /// Значения, с которыми игра работает: переопределения приходят снаружи (переменные, свойства
    /// Android, адрес страницы), и опечатка вроде `texture_max=0` или `compute=0` не должна вешать
    /// загрузку или ронять игру. Неверное значение отклоняется, как и неверный вид.
    fn check(&self) -> Result<(), String> {
        let finite = |value: f32| value.is_finite();
        for density in self.render.density.0.into_iter().flatten() {
            if !finite(density) || !(0.0..=4.0).contains(&density) {
                return Err(format!("render.density {density}: нужно от 0 до 4"));
            }
        }
        if ![1, 2, 4, 8].contains(&self.render.msaa_high) {
            return Err(format!("render.msaa_high {}: нужно 1, 2, 4 или 8", self.render.msaa_high));
        }
        for shadows in self.render.shadows.0 {
            if !(256..=8192).contains(&shadows.map) || !shadows.map.is_power_of_two() {
                return Err(format!("тени: карта {} — степень двойки от 256 до 8192", shadows.map));
            }
            if !(1..=4).contains(&shadows.cascades) {
                return Err(format!("тени: каскадов {} — от 1 до 4", shadows.cascades));
            }
            if !finite(shadows.far) || !(1.0..=1000.0).contains(&shadows.far) {
                return Err(format!("тени: дальность {} — от 1 до 1000 м", shadows.far));
            }
        }
        if let Some(fps) = self.render.frame_limit
            && (!finite(fps) || !(fps == 0.0 || (10.0..=480.0).contains(&fps)))
        {
            return Err(format!("render.frame_limit {fps}: 0 (без предела) или от 10 до 480"));
        }
        if !finite(self.debris.shadow_min_size) || self.debris.shadow_min_size < 0.0 {
            return Err(format!("debris.shadow_min_size {}: нужно от 0", self.debris.shadow_min_size));
        }
        if !finite(self.models.detail) || !(0.01..=1.0).contains(&self.models.detail) {
            return Err(format!("models.detail {}: нужно от 0,01 до 1", self.models.detail));
        }
        if let Some(max) = self.models.texture_max
            && !(64..=16384).contains(&max)
        {
            return Err(format!("models.texture_max {max}: нужно от 64 до 16384"));
        }
        /// Больше потоков пулу не нужно ни на одной машине: опечатка не должна плодить тысячи.
        const MAX_THREADS: usize = 256;
        if let Some(compute) = self.threads.compute
            && !(1..=MAX_THREADS).contains(&compute)
        {
            return Err(format!("threads.compute {compute}: нужно от 1 до {MAX_THREADS}"));
        }
        if let Some([min, max]) = self.threads.async_compute
            && (min == 0 || min > max || max > MAX_THREADS)
        {
            return Err(format!(
                "threads.async_compute [{min}, {max}]: нужно 1 ≤ наименьшее ≤ наибольшее ≤ {MAX_THREADS}"
            ));
        }
        Ok(())
    }

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
        for rejected in &profile.rejected {
            warn!("Профиль платформы: переопределение не принято: {rejected}");
        }
        app.insert_resource(profile.clone());
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(fps) = profile.render.frame_limit.filter(|fps| *fps > 0.0) {
                info!("Предел кадров: {fps} в секунду");
            }
            app.add_systems(Last, limit_frames);
        }
    }
}

/// Простой ограничитель кадров (`render.frame_limit`): в конце кадра поток досыпает до длительности
/// кадра. Нужен там, где развёртка быстрее нужного: iPad с ProMotion даёт 120 Гц, а держать 120
/// кадров игре незачем — греется и садит батарею. iPhone без ключа `CADisableMinimumFrameDurationOnPhone`
/// в `Info.plist` и так не быстрее 60 Гц. В браузере спать нельзя, там кадры задаёт страница.
/// Прогон `--bench` меряет, сколько игра может, поэтому предела в нём нет.
#[cfg(not(target_arch = "wasm32"))]
fn limit_frames(bench: Option<Res<crate::perf::Bench>>, mut last: Local<Option<bevy::platform::time::Instant>>) {
    let limit = if bench.is_some_and(|bench| bench.on) { None } else { frame_limit(get().render.frame_limit, overheated()) };
    let Some(fps) = limit else {
        *last = None;
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
        let all = profiles();
        let platform = platform_profile().to_owned();
        let mut rejected = Vec::new();
        // Профиль целиком из переопределения; опечатка в имени — профиль платформы.
        let name = match debug_value("profile") {
            Some(name) if all.contains_key(name.trim()) => name.trim().to_owned(),
            Some(name) => {
                rejected.push(format!("profile = {name} (нет такого профиля)"));
                platform
            }
            None => platform,
        };
        let resolved = resolve(&all, &name, &debug_value)
            .unwrap_or_else(|error| panic!("Профиль платформы «{name}»: {error}"));
        let mut profile: PlatformProfile = serde_json::from_value(resolved.value)
            .unwrap_or_else(|error| panic!("Профиль платформы «{name}»: {error}"));
        profile.name = name;
        profile.overrides = resolved.applied;
        // Переопределения вне профиля — тоже в строку профиля: всё действующее видно в одном месте.
        for key in ["quality", "off", "perf"] {
            if let Some(value) = debug_value(key) {
                let value = value.trim();
                // Ступень — только из трёх (её разбирает `quality.rs`, неверная там не действует).
                if key == "quality" && serde_json::from_value::<Quality>(Value::String(value.to_owned())).is_err() {
                    rejected.push(format!("quality = {value} (нужно low, medium или high)"));
                } else {
                    profile.overrides.push(format!("{key} = {value}"));
                }
            }
        }
        rejected.extend(resolved.rejected);
        profile.rejected = rejected;
        profile
    })
}

/// Слой нагрева: устройство горячее или в режиме энергосбережения (на iOS — `ios::watch_thermal`,
/// на Android — `android::watch_thermal` и Game Mode).
/// Тогда плотность кадра ниже на [`THROTTLE_DENSITY`] (`render_scale.rs`).
static THROTTLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Во сколько раз плотнее кадр, когда устройство горячее: на шаг ниже, как от medium к low.
pub const THROTTLE_DENSITY: f32 = 0.8;

pub fn throttled() -> bool {
    THROTTLED.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg_attr(not(any(target_os = "ios", target_os = "android")), allow(dead_code))]
pub fn set_throttled(on: bool) {
    THROTTLED.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Нагрев «critical»: кадров не больше [`OVERHEAT_FPS`], пока устройство не остынет (`ios.rs`,
/// `android.rs`).
static OVERHEATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Предел кадров при критическом нагреве.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub const OVERHEAT_FPS: f32 = 30.0;

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn overheated() -> bool {
    OVERHEATED.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg_attr(not(any(target_os = "ios", target_os = "android")), allow(dead_code))]
pub fn set_overheated(on: bool) {
    OVERHEATED.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Действующий предел кадров: из профиля, а при критическом нагреве — не больше [`OVERHEAT_FPS`].
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn frame_limit(profile: Option<f32>, overheated: bool) -> Option<f32> {
    let profile = profile.filter(|fps| *fps > 0.0);
    if overheated {
        Some(profile.map_or(OVERHEAT_FPS, |fps| fps.min(OVERHEAT_FPS)))
    } else {
        profile
    }
}

/// Предел кадров сейчас: из профиля и с учётом нагрева. `None` — без предела.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn current_frame_limit() -> Option<f32> {
    frame_limit(get().render.frame_limit, overheated())
}

/// Мало памяти до предела процесса (`ios.rs`) или у системы (`android.rs`): лежащих обломков вдвое меньше (`debris.rs`).
static LOW_MEMORY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn low_memory() -> bool {
    LOW_MEMORY.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg_attr(not(any(target_os = "ios", target_os = "android")), allow(dead_code))]
pub fn set_low_memory(on: bool) {
    LOW_MEMORY.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Уровень устройства iOS: по нему выбирается профиль внутри класса (iPhone, iPad).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceTier {
    Low,
    Mid,
    High,
}

/// Уровень по возможностям, а не по модели: семейство GPU Metal (`Apple<N>`: A12 — 5, A13 — 6,
/// A14 и M1 — 7, A17 Pro и M3 — 9), объём памяти и сколько до предела памяти процесса при запуске
/// (0 — неизвестно, как в симуляторе). Семейства накопительные, так что неизвестное новое устройство
/// попадает в сильные.
#[cfg_attr(not(any(target_os = "ios", test)), allow(dead_code))]
pub fn device_tier(apple_family: u32, physical: u64, available: u64, tablet: bool) -> DeviceTier {
    const GIB: u64 = 1 << 30;
    // До предела процесса меньше 1,8 ГБ — устройство с 3 ГБ памяти, какой бы ни был GPU.
    let tight = available > 0 && available < GIB * 18 / 10;
    // Устройства с 8 ГБ сообщают чуть меньше 8 ГиБ.
    let big = physical >= GIB * 7;
    if apple_family < 7 || tight {
        DeviceTier::Low
    } else if big && (apple_family >= 9 || tablet) {
        DeviceTier::High
    } else {
        DeviceTier::Mid
    }
}

/// Имя профиля iOS по уровню и классу устройства.
#[cfg_attr(not(any(target_os = "ios", test)), allow(dead_code))]
fn ios_profile(tier: DeviceTier, tablet: bool) -> &'static str {
    match (tablet, tier) {
        (false, DeviceTier::Low) => "ios-phone-low",
        (false, DeviceTier::Mid) => "ios-phone",
        (false, DeviceTier::High) => "ios-phone-high",
        (true, DeviceTier::Low) => "ios-tablet-low",
        (true, DeviceTier::Mid) => "ios-tablet",
        (true, DeviceTier::High) => "ios-tablet-high",
    }
}

/// Уровень устройства Android по имени GPU адаптера Vulkan, объёму памяти (`MemTotal`, 0 — неизвестно)
/// и признаку слабого устройства `ro.config.low_ram`. Номер Adreno не растёт с мощностью (810 слабее
/// 740), поэтому флагманы узнаются по второй цифре: 730, 740, 750, 830, 840. Неизвестный GPU — средний:
/// у имён Android-GPU порядка нет, в отличие от семейств Metal.
#[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
pub fn android_tier(gpu: &str, memory: u64, low_ram: bool) -> DeviceTier {
    const GIB: u64 = 1 << 30;
    // Телефон с 4 ГБ сообщает около 3,6 ГиБ, с 6 ГБ — около 5,4, с 8 ГБ — около 7,2.
    let small = low_ram || (memory > 0 && memory < GIB * 5);
    let big = memory >= GIB * 13 / 2;
    // Номер модели после приставки: «Adreno (TM) 642L» → 642, «Mali-G715 MC11» → 715.
    let model = |prefix: &str| -> Option<u32> {
        let rest = gpu.find(prefix).map(|at| &gpu[at + prefix.len()..])?;
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    };
    let weak_gpu = model("Adreno (TM) ").is_some_and(|n| n < 700)
        // Mali-G31…G78 — Bifrost и ранний Valhall; G310 и новее — трёхзначные; с 2025 года —
        // Mali-G1-Ultra/Premium/Pro (Arm Lumex), номер 1. Mali-T — ещё старее.
        || model("Mali-G").is_some_and(|n| (30..100).contains(&n))
        || gpu.contains("Mali-T")
        || gpu.contains("PowerVR");
    let strong_gpu = model("Adreno (TM) ").is_some_and(|n| (700..900).contains(&n) && n / 10 % 10 >= 3)
        || gpu.contains("Immortalis")
        || gpu.contains("G1-Ultra")
        || model("Xclipse ").is_some_and(|n| (900..1000).contains(&n));
    if small || weak_gpu {
        DeviceTier::Low
    } else if big && strong_gpu {
        DeviceTier::High
    } else {
        DeviceTier::Mid
    }
}

/// Имя профиля Android по уровню.
#[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
fn android_profile(tier: DeviceTier) -> &'static str {
    match tier {
        DeviceTier::Low => "android-low",
        DeviceTier::Mid => "android",
        DeviceTier::High => "android-high",
    }
}

/// Нагрев устройства по запасу до троттлинга: слой поверх профиля.
#[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Heat {
    Cool,
    /// Кадр на шаг реже ([`THROTTLE_DENSITY`]).
    Throttled,
    /// Не больше [`OVERHEAT_FPS`] кадров.
    Overheated,
}

/// Нагрев по запасу до троттлинга Android (`AThermal_getThermalHeadroom`: 0 — холодный, 1,0 —
/// уровень SEVERE). Рекомендация Google: от 0,95 снижать нагрузку, от 1,0 — сразу. Назад — на 0,1
/// ниже порога, иначе у порога кадр меняется каждые 10 с.
#[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
pub fn heat_from_headroom(headroom: f32, was: Heat) -> Heat {
    let level = |throttled: f32, overheated: f32| {
        if headroom >= overheated {
            Heat::Overheated
        } else if headroom >= throttled {
            Heat::Throttled
        } else {
            Heat::Cool
        }
    };
    let up = level(0.95, 1.0);
    let down = level(0.85, 0.9);
    up.max(was.min(down))
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
        android_profile(crate::android::device().tier())
    }
    #[cfg(target_os = "ios")]
    {
        let device = crate::ios::device();
        ios_profile(device.tier(), device.tablet)
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

/// Профиль после наследования и переопределений.
struct Resolved {
    value: Value,
    /// Принятые переопределения: `путь = значение`.
    applied: Vec<String>,
    /// Отклонённые: значение не того вида — профиль с ним не разбирается.
    rejected: Vec<String>,
}

/// Профиль `name` после наследования и переопределений. Каждое переопределение проверяется
/// разбором профиля: неверное значение отклоняется, а не роняет игру.
fn resolve(
    all: &Map<String, Value>,
    name: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<Resolved, String> {
    let value = inherited(all, name, 0)?;
    serde_json::from_value::<PlatformProfile>(value.clone())
        .map_err(|error| error.to_string())?
        .check()?;
    let mut resolved = Resolved {
        value,
        applied: Vec::new(),
        rejected: Vec::new(),
    };
    let accept = |resolved: &mut Resolved,
                      label: String,
                      change: &dyn Fn(&mut Value) -> Result<(), String>| {
        let mut candidate = resolved.value.clone();
        let checked = change(&mut candidate).and_then(|()| {
            serde_json::from_value::<PlatformProfile>(candidate.clone())
                .map_err(|error| error.to_string())
                .and_then(|profile| profile.check())
        });
        match checked {
            Ok(_) => {
                resolved.value = candidate;
                resolved.applied.push(label);
            }
            Err(error) => resolved.rejected.push(format!("{label} ({error})")),
        }
    };
    // Старые имена — до путей: путь задаёт точнее и побеждает.
    for (alias, apply) in ALIASES {
        if let Some(text) = lookup(alias) {
            let text = text.trim().to_owned();
            accept(&mut resolved, format!("{alias} = {text}"), &|value| {
                apply(value, &text)
            });
        }
    }
    let mut paths = Vec::new();
    leaf_paths(&resolved.value, String::new(), &mut paths);
    for path in paths {
        if let Some(text) = lookup(&path) {
            let text = text.trim().to_owned();
            let parsed = serde_json::from_str(&text).unwrap_or(Value::String(text.clone()));
            accept(&mut resolved, format!("{path} = {text}"), &|value| {
                set_path(value, &path, parsed.clone());
                Ok(())
            });
        }
    }
    Ok(resolved)
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
        value["render"]["density"] = if density == 0.0 {
            Value::Null
        } else if density.is_finite() && density > 0.0 {
            density.into()
        } else {
            return Err(format!("density {text}: нужно 0 (экран как есть) или больше"));
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

    const ALL: [&str; 17] = [
        "base",
        "desktop",
        "macos",
        "linux",
        "windows",
        "web-desktop",
        "web-mobile",
        "mobile",
        "android",
        "android-low",
        "android-high",
        "ios-phone",
        "ios-tablet",
        "ios-phone-low",
        "ios-phone-high",
        "ios-tablet-low",
        "ios-tablet-high",
    ];

    fn build(name: &str, overrides: &[(&str, &str)]) -> PlatformProfile {
        let lookup = |key: &str| {
            overrides
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        };
        let resolved = resolve(&profiles(), name, &lookup).unwrap();
        let mut profile: PlatformProfile = serde_json::from_value(resolved.value).unwrap();
        profile.name = name.to_owned();
        profile.overrides = resolved.applied;
        profile.rejected = resolved.rejected;
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
    fn уровень_устройства_ios_по_возможностям() {
        const GB: u64 = 1 << 30;
        // iPad 9: A13, 3 ГБ.
        assert_eq!(device_tier(6, 3 * GB, 0, true), DeviceTier::Low);
        // iPhone 12: A14, 4 ГБ, до предела 2 ГБ.
        assert_eq!(device_tier(7, 4 * GB, 2 * GB, false), DeviceTier::Mid);
        // Новое семейство, но памяти мало — слабый.
        assert_eq!(device_tier(9, 8 * GB, GB, false), DeviceTier::Low);
        // iPhone 15 Pro Max: A17 Pro, 8 ГБ (система сообщает чуть меньше).
        assert_eq!(device_tier(9, 8 * GB - (300 << 20), 5 * GB, false), DeviceTier::High);
        // iPhone 15: A16, 6 ГБ.
        assert_eq!(device_tier(8, 6 * GB, 3 * GB, false), DeviceTier::Mid);
        // iPad Pro M1, 8 ГБ — сильный; iPhone на A14 с 8 ГБ не бывает, но был бы средним.
        assert_eq!(device_tier(7, 8 * GB, 5 * GB, true), DeviceTier::High);
        assert_eq!(device_tier(7, 8 * GB, 5 * GB, false), DeviceTier::Mid);
        // Неизвестное новое устройство.
        assert_eq!(device_tier(12, 16 * GB, 0, false), DeviceTier::High);
        assert_eq!(ios_profile(DeviceTier::Low, true), "ios-tablet-low");
        assert_eq!(ios_profile(DeviceTier::High, false), "ios-phone-high");
    }

    /// Слабый уровень легче среднего, средний легче сильного — по каждому числу нагрузки.
    #[test]
    fn уровни_ios_упорядочены() {
        for class in ["ios-phone", "ios-tablet"] {
            let [low, mid, high] =
                [format!("{class}-low"), class.to_owned(), format!("{class}-high")].map(|n| build(&n, &[]));
            assert_eq!((low.class, high.class), (mid.class, mid.class), "{class}");
            for q in [Quality::Low, Quality::Medium, Quality::High] {
                assert!(low.density(q) <= mid.density(q) && mid.density(q) <= high.density(q), "{class} {q:?}");
                let shadow = |p: &PlatformProfile| (p.shadows(q).map, p.shadows(q).cascades);
                assert!(shadow(&low) <= shadow(&mid) && shadow(&mid) <= shadow(&high), "{class} {q:?}");
                assert!(low.lights.fire.at(q) <= mid.lights.fire.at(q), "{class} {q:?}");
                assert!(mid.lights.fire.at(q) <= high.lights.fire.at(q), "{class} {q:?}");
            }
            assert!(low.debris.max_resting < mid.debris.max_resting, "{class}");
            assert!(mid.debris.max_resting < high.debris.max_resting, "{class}");
            assert!(low.lights.flashes <= mid.lights.flashes && mid.lights.flashes <= high.lights.flashes);
            assert!(low.models.texture_max < mid.models.texture_max, "{class}");
            assert!(low.render.msaa_high <= mid.render.msaa_high);
            let rank = |p: &PlatformProfile| p.default_quality as u8;
            assert!(rank(&low) <= rank(&mid) && rank(&mid) <= rank(&high), "{class}");
            // Не заданное у уровня — от профиля класса.
            assert_eq!((low.menu.quit, high.input.touch_default), (mid.menu.quit, mid.input.touch_default));
        }
    }

    #[test]
    fn уровень_устройства_android_по_gpu_и_памяти() {
        const GB: u64 = 1 << 30;
        let tier = |gpu: &str| android_tier(gpu, 8 * GB - (700 << 20), false);
        // Adreno 6xx и ниже — слабые, 720 и 810 — средние, 730/740/750/830 — сильные.
        assert_eq!(tier("Adreno (TM) 610"), DeviceTier::Low);
        assert_eq!(tier("Adreno (TM) 642L"), DeviceTier::Low);
        assert_eq!(tier("Adreno (TM) 720"), DeviceTier::Mid);
        assert_eq!(tier("Adreno (TM) 810"), DeviceTier::Mid);
        assert_eq!(tier("Adreno (TM) 740"), DeviceTier::High);
        assert_eq!(tier("Adreno (TM) 830"), DeviceTier::High);
        // Mali Bifrost и ранний Valhall — слабые, новые — средние, Immortalis — сильный.
        assert_eq!(tier("Mali-G57 MC2"), DeviceTier::Low);
        assert_eq!(tier("Mali-G78 MP14"), DeviceTier::Low);
        assert_eq!(tier("Mali-G715 MC7"), DeviceTier::Mid);
        assert_eq!(tier("Immortalis-G720 MC12"), DeviceTier::High);
        // Arm Lumex (Dimensity 9500 и новее): Ultra — сильный, Premium и Pro — средние.
        assert_eq!(tier("Mali-G1-Ultra MC12"), DeviceTier::High);
        assert_eq!(tier("Mali-G1-Premium MC10"), DeviceTier::Mid);
        assert_eq!(tier("Mali-G1-Pro MC6"), DeviceTier::Mid);
        assert_eq!(tier("Samsung Xclipse 940"), DeviceTier::High);
        assert_eq!(tier("Samsung Xclipse 540"), DeviceTier::Mid);
        assert_eq!(tier("PowerVR B-Series BXM-8-256"), DeviceTier::Low);
        // Неизвестный GPU — средний, даже с большой памятью.
        assert_eq!(tier("Vulkan GPU 2030"), DeviceTier::Mid);
        assert_eq!(android_tier("Vulkan GPU 2030", 16 * GB, false), DeviceTier::Mid);
        // Флагман, но мало памяти или система считает его слабым.
        assert_eq!(android_tier("Adreno (TM) 830", 4 * GB, false), DeviceTier::Low);
        assert_eq!(android_tier("Adreno (TM) 830", 6 * GB, false), DeviceTier::Mid);
        assert_eq!(android_tier("Adreno (TM) 830", 12 * GB, true), DeviceTier::Low);
        // Память неизвестна — решает GPU.
        assert_eq!(android_tier("Adreno (TM) 810", 0, false), DeviceTier::Mid);
        assert_eq!(android_profile(DeviceTier::Low), "android-low");
        assert_eq!(android_profile(DeviceTier::Mid), "android");
        assert_eq!(android_profile(DeviceTier::High), "android-high");
    }

    /// Слабый Android легче среднего, средний легче сильного; средний — прежние числа и предел 60.
    #[test]
    fn уровни_android_упорядочены() {
        let [low, mid, high] = ["android-low", "android", "android-high"].map(|n| build(n, &[]));
        for q in [Quality::Low, Quality::Medium, Quality::High] {
            assert!(low.density(q) <= mid.density(q) && mid.density(q) <= high.density(q), "{q:?}");
            let shadow = |p: &PlatformProfile| (p.shadows(q).map, p.shadows(q).cascades);
            assert!(shadow(&low) <= shadow(&mid) && shadow(&mid) <= shadow(&high), "{q:?}");
            assert!(low.lights.fire.at(q) <= mid.lights.fire.at(q), "{q:?}");
            assert!(mid.lights.fire.at(q) <= high.lights.fire.at(q), "{q:?}");
            assert!(low.render.bloom.at(q) <= mid.render.bloom.at(q), "{q:?}");
        }
        assert!(low.debris.max_resting < mid.debris.max_resting);
        assert!(mid.debris.max_resting < high.debris.max_resting);
        assert!(low.lights.flashes <= mid.lights.flashes && mid.lights.flashes <= high.lights.flashes);
        assert!(low.models.texture_max < mid.models.texture_max);
        assert_eq!(mid.models.texture_max, high.models.texture_max);
        assert_eq!((low.render.msaa_high, mid.render.msaa_high, high.render.msaa_high), (1, 1, 1));
        assert_eq!(
            (low.render.frame_limit, mid.render.frame_limit, high.render.frame_limit),
            (Some(30.0), Some(60.0), Some(60.0))
        );
        // Не заданное у уровня — от android: потоки, ожидание конвейеров, касание.
        for p in [&low, &high] {
            assert_eq!((p.threads.compute, p.prewarm.wait_pipelines), (mid.threads.compute, true));
            assert!(p.input.touch_default && p.audio.pause_in_background);
        }
    }

    #[test]
    fn нагрев_android_по_запасу_с_гистерезисом() {
        use Heat::*;
        assert_eq!(heat_from_headroom(0.5, Cool), Cool);
        assert_eq!(heat_from_headroom(0.95, Cool), Throttled);
        assert_eq!(heat_from_headroom(1.0, Cool), Overheated);
        assert_eq!(heat_from_headroom(1.3, Throttled), Overheated);
        // Остывание — на 0,1 ниже порога.
        assert_eq!(heat_from_headroom(0.92, Overheated), Overheated);
        assert_eq!(heat_from_headroom(0.89, Overheated), Throttled);
        assert_eq!(heat_from_headroom(0.80, Overheated), Cool);
        assert_eq!(heat_from_headroom(0.90, Throttled), Throttled);
        assert_eq!(heat_from_headroom(0.84, Throttled), Cool);
    }

    #[test]
    fn предел_кадров_при_нагреве() {
        assert_eq!(frame_limit(None, false), None);
        assert_eq!(frame_limit(Some(60.0), false), Some(60.0));
        assert_eq!(frame_limit(Some(0.0), false), None);
        assert_eq!(frame_limit(None, true), Some(OVERHEAT_FPS));
        assert_eq!(frame_limit(Some(60.0), true), Some(OVERHEAT_FPS));
        assert_eq!(frame_limit(Some(20.0), true), Some(20.0));
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
        assert_eq!(tablet.density(Quality::Medium), Some(1.5));
        assert_eq!(tablet.default_quality, Quality::Medium);
        assert_eq!(tablet.render.frame_limit, Some(60.0));
        assert_eq!(tablet.debris, phone.debris);
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
    fn неверное_переопределение_отклоняется() {
        let p = build(
            "android",
            &[
                ("render.density", "abc"),
                ("shadows", "x"),
                ("debris.max_resting", "50"),
            ],
        );
        assert_eq!(p.density(Quality::Low), Some(1.25));
        assert_eq!(p.shadows(Quality::Medium).cascades, 1);
        assert_eq!(p.debris.max_resting, 50);
        assert_eq!(p.rejected.len(), 2);
        assert_eq!(p.overrides, vec!["debris.max_resting = 50".to_owned()]);
    }

    /// Значения, на которых игра зависла бы или упала, отклоняются: профиль остаётся прежним.
    #[test]
    fn опасное_переопределение_отклоняется() {
        let bad = [
            ("models.texture_max", "0"),
            ("threads.compute", "0"),
            ("threads.async_compute", "[6, 2]"),
            ("shadows", "0,60"),
            ("render.frame_limit", "1e-40"),
            ("render.density", "-1"),
            ("render.msaa_high", "3"),
            ("models.detail", "0"),
            ("density", "-1"),
            ("density", "NaN"),
            ("density", "inf"),
            ("threads.compute", "10000"),
            ("compute", "10000"),
        ];
        let base = build("android", &[]);
        for (key, value) in bad {
            let p = build("android", &[(key, value)]);
            assert_eq!(p.rejected.len(), 1, "{key} = {value}");
            assert!(p.overrides.is_empty(), "{key} = {value}");
            assert_eq!((&p.render, &p.models, &p.threads), (&base.render, &base.models, &base.threads), "{key} = {value}");
        }
        // Пределы, которые имеют смысл, принимаются.
        let p = build("android", &[("render.frame_limit", "0"), ("models.texture_max", "null"), ("threads.async_compute", "[1, 1]")]);
        assert!(p.rejected.is_empty(), "{:?}", p.rejected);
        // Каждый профиль файла проходит проверку.
        for name in ALL {
            build(name, &[]).check().unwrap_or_else(|error| panic!("{name}: {error}"));
        }
    }

    #[test]
    fn имена_переменных() {
        assert_eq!(env_name("render.density"), "SPACE_GUARD_RENDER_DENSITY");
        assert_eq!(env_name("off"), "SPACE_GUARD_OFF");
        assert_eq!(env_name("detail"), "SPACE_GUARD_DETAIL");
    }
}
