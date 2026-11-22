//! Сенсорное управление: телефон и планшет (Android и браузер).
//!
//! Схема экрана — https://claude.ai/artifact/GctBfPbonNnJSF1BT2s9kg (копия в изменении OpenSpec
//! `mobile-touch-controls`, `touch-layout.html`).
//!
//! Касания пишут в те же входы, что геймпад: езда и прицел — в [`PadInput`], огонь — левая кнопка
//! мыши, кнопки — клавиши. Поэтому оружие, пауза и наведение авиаудара работают без своих веток.
//! Система стоит в `PreUpdate` после `gamepad::read_pad`, который каждый кадр заново пишет
//! `PadInput`.
//!
//! Каждый палец закреплён за тем, на чём начал, до отпускания: кнопкой интерфейса с [`TapAction`],
//! стиком езды (левая область), стиком прицела (правая область), точкой попадания или отметкой
//! авиаудара. Кнопки важнее областей. У мортиры и ракет стика прицела нет: палец вне стика езды
//! указывает, куда бить, и держит огонь, пока лежит на экране. Касание вне всего этого — «ничьё»:
//! короткое касание даёт [`TouchTap`] (экраны вне забега ищут под ним строку), горизонтальный
//! свайп — [`TouchSwipe`].
//!
//! Размеры — в dp: на Android логическая точка окна — dp, в браузере на телефоне холст рисует
//! в `platform::canvas_scale` раз меньше точек, чем у страницы.

use std::collections::HashMap;

use bevy::input::InputSystems;
use bevy::input::touch::Touches;
use bevy::prelude::*;
use bevy::ui::{ComputedStackIndex, UiGlobalTransform};
use bevy::window::PrimaryWindow;

use crate::airstrike::Targeting;
use crate::camera::GameCamera;
use crate::frame::GameSet;
use crate::gamepad::{InputDevice, PadInput, deadzone, screen_axes, to_world};
use crate::rover::Rover;
use crate::run::GameState;
use crate::settings::Settings;
use crate::terrain::Terrain;
use crate::weapons::CurrentWeapon;

/// Числа раскладки в dp. Начальные значения со схемы, подбираются на телефоне.
pub mod layout {
    /// Область езды — левая часть экрана, доля ширины.
    pub const DRIVE_ZONE_RIGHT: f32 = 0.42;
    /// Область прицела — правая часть экрана, от этой доли ширины.
    pub const AIM_ZONE_LEFT: f32 = 0.6;
    /// Обе области ниже этой доли высоты: верх занят интерфейсом.
    pub const ZONE_TOP: f32 = 0.25;
    /// Радиус стика езды и прицела, dp.
    pub const DRIVE_RADIUS: f32 = 60.0;
    pub const AIM_RADIUS: f32 = 62.0;
    /// Радиус рукояти, dp.
    pub const KNOB_RADIUS: f32 = 26.0;
    /// Место стиков по умолчанию: от левого (правого) и нижнего края, dp.
    pub const DRIVE_HOME: (f32, f32) = (120.0, 98.0);
    pub const AIM_HOME: (f32, f32) = (110.0, 98.0);
    /// Мёртвая зона стика езды, доля радиуса.
    pub const DRIVE_DEADZONE: f32 = 0.12;
    /// Оттяжка стика прицела, после которой прицел и огонь, доля радиуса.
    pub const AIM_DEADZONE: f32 = 0.2;
    /// Короткое касание: не дольше, с, и не дальше доли радиуса прицела.
    pub const TAP_SECONDS: f32 = 0.25;
    pub const TAP_SLOP: f32 = 0.15;
    /// Сколько держится огонь после короткого касания, с.
    pub const TAP_FIRE_SECONDS: f32 = 0.2;
    /// Отступ от краёв экрана, dp.
    pub const MARGIN: f32 = 16.0;
    /// Кнопка паузы, dp.
    pub const PAUSE_SIZE: f32 = 44.0;
    /// Кнопки наведения авиаудара: поворот, «Удар», «Отмена» (ширина × высота), dp.
    pub const TURN_SIZE: f32 = 52.0;
    pub const STRIKE_SIZE: f32 = 92.0;
    pub const CANCEL_SIZE: (f32, f32) = (104.0, 40.0);
    /// Касание строки меню засчитывается, если до неё не дальше, dp.
    pub const ROW_SLOP: f32 = 14.0;
    /// Свайп: горизонтальный сдвиг не меньше, dp.
    pub const SWIPE: f32 = 60.0;
    /// Поворот серии авиаудара двумя пальцами: шаг, рад (15°).
    pub const TWIST_STEP: f32 = std::f32::consts::PI / 12.0;
    /// Сколько секунд после касания мышь не переключает устройство: браузер может прислать
    /// мышиные события, сделанные из касания.
    pub const MOUSE_AFTER_TOUCH: f64 = 0.5;
}

use layout::*;

/// Плагин сенсорного управления.
pub struct TouchPlugin;

impl Plugin for TouchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TouchControls>()
            .init_resource::<TouchTap>()
            .init_resource::<TouchSwipe>()
            .init_resource::<LastTouch>()
            .add_systems(Startup, spawn_overlay)
            // С `--touch` и на телефонах (`input.touch_default` профиля: Android, iPhone, iPad) раскладка
            // касаний видна сразу, до первого нажатия, а вступление и обучение не говорят о клавиатуре.
            .add_systems(
                Startup,
                (|mut device: ResMut<InputDevice>| *device = InputDevice::Touch)
                    .run_if(|| mouse_touch_enabled() || crate::profile::get().input.touch_default),
            )
            .add_systems(
                PreUpdate,
                (
                    mouse_as_touch.before(InputSystems),
                    drop_mouse_click.after(InputSystems).before(read_touches),
                )
                    .run_if(mouse_touch_enabled),
            )
            .add_systems(
                PreUpdate,
                release_on_blur.after(InputSystems).before(read_touches),
            )
            .add_systems(
                PreUpdate,
                read_touches
                    .after(InputSystems)
                    .after(crate::gamepad::PadRead)
                    .before(crate::intro::skip_intro),
            )
            .add_systems(Update, show_overlay.in_set(GameSet::Presentation));
    }
}

/// Отладка без сенсорного экрана: `--touch` (или `SPACE_GUARD_TOUCH=1`) — левая кнопка мыши
/// изображает палец.
fn mouse_touch_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::args().any(|arg| arg == "--touch")
            || std::env::var_os("SPACE_GUARD_TOUCH").is_some()
    })
}

/// Мышь как палец: нажатие, движение с зажатой кнопкой и отпускание — сообщения касания.
fn mouse_as_touch(
    mut buttons: MessageReader<bevy::input::mouse::MouseButtonInput>,
    window: Query<(Entity, &Window), With<PrimaryWindow>>,
    mut held: Local<bool>,
    mut touches: MessageWriter<bevy::input::touch::TouchInput>,
) {
    use bevy::input::ButtonState;
    use bevy::input::touch::{TouchInput, TouchPhase};
    let Ok((entity, window)) = window.single() else {
        return;
    };
    let Some(position) = window.cursor_position() else {
        return;
    };
    let send = |phase| TouchInput {
        phase,
        position,
        window: entity,
        force: None,
        id: 0,
    };
    let mut phase = None;
    for event in buttons.read().filter(|e| e.button == MouseButton::Left) {
        match event.state {
            ButtonState::Pressed if !*held => {
                *held = true;
                phase = Some(TouchPhase::Started);
            }
            ButtonState::Released if *held => {
                *held = false;
                phase = Some(TouchPhase::Ended);
            }
            _ => {}
        }
    }
    match phase {
        Some(phase) => {
            touches.write(send(phase));
        }
        None if *held => {
            touches.write(send(TouchPhase::Moved));
        }
        None => {}
    }
}

/// Щелчок мыши, ставший касанием, не стреляет сам: огонь даёт стик прицела.
fn drop_mouse_click(mut mouse: ResMut<ButtonInput<MouseButton>>) {
    if mouse.just_pressed(MouseButton::Left) {
        mouse.reset(MouseButton::Left);
    }
}

/// Что делает касание узла интерфейса. Узел с этим компонентом — кнопка: палец, начавший на нём,
/// держит клавишу или кнопку мыши до отпускания.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapAction {
    Key(KeyCode),
    Mouse(MouseButton),
    /// Шаг по оружию или поворот серии авиаудара (`PadInput::weapon_step`).
    Step(i32),
}

/// Короткое касание вне кнопок и стиков в этом кадре: точка в логических точках окна.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct TouchTap(pub Option<Vec2>);

/// Горизонтальный свайп вне кнопок и стиков в этом кадре: −1 влево, 1 вправо, 0 — нет.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct TouchSwipe(pub i32);

/// Когда было последнее касание, секунды реального времени; `None` — касаний не было.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct LastTouch(pub Option<f64>);

impl LastTouch {
    /// Касание было только что: мышиные события сейчас, скорее всего, сделаны из него.
    pub fn recent(&self, now: f64) -> bool {
        self.0.is_some_and(|at| now - at < MOUSE_AFTER_TOUCH)
    }
}

/// Чему принадлежит палец.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Control {
    /// Кнопка интерфейса.
    Button(TapAction),
    Drive,
    Aim,
    /// Отметка авиаудара в режиме наведения.
    Strike,
    /// Точка попадания мортиры и ракет: палец указывает, куда бить.
    Point,
    /// Ни кнопка, ни стик: короткое касание — `TouchTap`, свайп — `TouchSwipe`.
    Free,
    /// Палец, оборвавший вступление: ничего не делает до отпускания.
    Spent,
}

/// Палец на экране.
#[derive(Debug, Clone, Copy)]
struct Finger {
    control: Control,
    /// Где и когда коснулся, секунды реального времени.
    start: Vec2,
    started: f64,
    /// Центр стика: у езды едет за пальцем.
    base: Vec2,
    /// Палец уходил дальше короткого касания.
    moved: bool,
}

/// Состояние сенсорного управления: пальцы и то, что они держат.
#[derive(Resource, Default)]
pub struct TouchControls {
    fingers: HashMap<u64, Finger>,
    /// Нажатые пальцами клавиши и кнопки мыши: отпускается ровно то, что нажато.
    held: Vec<(u64, TapAction)>,
    /// Огонь короткого касания: сколько секунд ещё держать левую кнопку.
    tap_fire: f32,
    /// Огонь оттяжкой стика прицела держит левую кнопку.
    aim_fire: bool,
    /// Последняя точка попадания, указанная пальцем (x, z): держится, пока идёт огонь касания.
    point: Option<Vec2>,
    /// Угол между двумя пальцами в наведении авиаудара, не отданный шагами поворота.
    twist: Option<f32>,
    /// Стики сейчас на экране: центр и рукоять (логические точки), для экранных элементов.
    pub drive_stick: Option<(Vec2, Vec2)>,
    pub aim_stick: Option<(Vec2, Vec2)>,
}

/// Раскладка экрана в логических точках окна.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub size: Vec2,
    /// Логических точек в одном dp.
    pub unit: f32,
    /// Множитель размера из настроек.
    pub scale: f32,
    /// Отступы от вырезов экрана: слева, сверху, справа, снизу, логические точки.
    pub inset: Vec4,
}

impl Layout {
    pub fn new(size: Vec2, settings: &Settings) -> Self {
        let unit = 1.0 / crate::platform::canvas_scale();
        let safe = crate::platform::safe_area() * unit;
        Self {
            size,
            unit,
            scale: settings.touch_size.factor(),
            inset: safe,
        }
    }

    /// dp элементов управления (с множителем размера) в логические точки.
    pub fn dp(&self, value: f32) -> f32 {
        value * self.unit * self.scale
    }

    fn left(&self) -> f32 {
        self.inset.x + MARGIN * self.unit
    }

    fn right(&self) -> f32 {
        self.size.x - self.inset.z - MARGIN * self.unit
    }

    fn top(&self) -> f32 {
        self.inset.y + MARGIN * self.unit
    }

    fn bottom(&self) -> f32 {
        self.size.y - self.inset.w - MARGIN * self.unit
    }

    pub fn drive_radius(&self) -> f32 {
        self.dp(DRIVE_RADIUS)
    }

    pub fn aim_radius(&self) -> f32 {
        self.dp(AIM_RADIUS)
    }

    /// Место стика езды по умолчанию.
    pub fn drive_home(&self) -> Vec2 {
        Vec2::new(
            self.inset.x + self.dp(DRIVE_HOME.0),
            self.size.y - self.inset.w - self.dp(DRIVE_HOME.1),
        )
    }

    /// Место стика прицела по умолчанию.
    pub fn aim_home(&self) -> Vec2 {
        Vec2::new(
            self.size.x - self.inset.z - self.dp(AIM_HOME.0),
            self.size.y - self.inset.w - self.dp(AIM_HOME.1),
        )
    }

    fn in_drive_zone(&self, point: Vec2) -> bool {
        point.x < self.size.x * DRIVE_ZONE_RIGHT && point.y > self.size.y * ZONE_TOP
    }

    fn in_aim_zone(&self, point: Vec2) -> bool {
        point.x > self.size.x * AIM_ZONE_LEFT && point.y > self.size.y * ZONE_TOP
    }

    /// Центр стика под пальцем: стик целиком на экране.
    fn stick_base(&self, point: Vec2, radius: f32) -> Vec2 {
        let min = Vec2::new(self.left() + radius, self.top() + radius);
        let max = Vec2::new(self.right() - radius, self.bottom() - radius).max(min);
        point.clamp(min, max)
    }
}

/// Оружие, которое бьёт в указанную пальцем точку, а не по стику прицела.
pub fn points_at(weapon: CurrentWeapon) -> bool {
    matches!(
        weapon,
        CurrentWeapon::Mortar | CurrentWeapon::Rockets | CurrentWeapon::HomingRockets
    )
}

/// Чему достаётся новый палец: кнопка под ним, иначе стик своей области, в наведении авиаудара —
/// отметка. У оружия с точкой попадания (`pointing`) всё, кроме стика езды, — точка; второй палец
/// в области езды при уже взятом стике (`driving`) — тоже. Вне забега стиков нет.
fn hit(
    point: Vec2,
    layout: &Layout,
    button: Option<TapAction>,
    playing: bool,
    targeting: bool,
    pointing: bool,
    driving: bool,
) -> Control {
    if let Some(action) = button {
        return Control::Button(action);
    }
    if !playing {
        return Control::Free;
    }
    if targeting {
        return Control::Strike;
    }
    if layout.in_drive_zone(point) && !(pointing && driving) {
        Control::Drive
    } else if pointing {
        Control::Point
    } else if layout.in_aim_zone(point) {
        Control::Aim
    } else {
        Control::Free
    }
}

/// Наклон стика: сдвиг пальца от центра в долях радиуса, ось Y вверх, длина до 1.
fn stick_tilt(base: Vec2, point: Vec2, radius: f32) -> Vec2 {
    let offset = (point - base) / radius.max(1.0);
    Vec2::new(offset.x, -offset.y).clamp_length_max(1.0)
}

/// Центр плавающего стика за пальцем: палец дальше края тянет центр за собой.
fn follow_base(base: Vec2, point: Vec2, radius: f32) -> Vec2 {
    let offset = point - base;
    let length = offset.length();
    if length <= radius {
        return base;
    }
    point - offset / length * radius
}

/// Доля дальности мортиры по оттяжке: от мёртвой зоны до края — от 0 до 1.
pub fn range_fraction(tilt: f32) -> f32 {
    ((tilt - AIM_DEADZONE) / (1.0 - AIM_DEADZONE)).clamp(0.0, 1.0)
}

/// Кнопка интерфейса под точкой: верхняя видимая с [`TapAction`].
fn button_at(
    point: Vec2,
    scale: f32,
    buttons: &Query<(
        &TapAction,
        &ComputedNode,
        &UiGlobalTransform,
        &InheritedVisibility,
        Option<&ComputedStackIndex>,
    )>,
) -> Option<TapAction> {
    buttons
        .iter()
        .filter(|(_, node, transform, visible, _)| {
            visible.get()
                && node.size().min_element() > 0.0
                && node_contains(node, transform, point, scale)
        })
        .max_by_key(|(.., stack)| stack.map_or(0, |s| s.0))
        .map(|(action, ..)| *action)
}

/// Сколько точек интерфейса (`Val::Px` с учётом `UiScale`) в `dp` dp.
pub fn dp_to_ui(dp: f32, ui_scale: &UiScale) -> f32 {
    dp / crate::platform::canvas_scale() / ui_scale.0.max(f32::EPSILON)
}

/// Место кнопки паузы у правого края (с отступом до соседа), пиксели интерфейса: кнопка растёт
/// с размером из настроек, мини-карта и название оружия при касаниях стоят левее неё.
pub fn pause_footprint_ui(settings: &Settings, ui_scale: &UiScale) -> f32 {
    dp_to_ui(PAUSE_SIZE * settings.touch_size.factor() + MARGIN, ui_scale)
}

/// Строка под точкой или ближайшая к ней в пределах [`ROW_SLOP`] dp: строки меню мельче пальца.
/// `point` — касание в логических точках окна, `scale` — масштаб окна (физических пикселей в точке).
pub fn nearest_row<'a, T: Copy>(
    point: Vec2,
    scale: f32,
    nodes: impl Iterator<Item = (T, &'a ComputedNode, &'a UiGlobalTransform)>,
) -> Option<T> {
    // Узлы интерфейса — в физических пикселях окна.
    let point = point * scale;
    let slop = ROW_SLOP / crate::platform::canvas_scale() * scale;
    nodes
        .filter(|(_, node, _)| node.size().min_element() > 0.0)
        .map(|(row, node, transform)| {
            let center = transform.translation;
            let half = node.size() * 0.5;
            (
                row,
                ((point - center).abs() - half).max(Vec2::ZERO).length(),
            )
        })
        .filter(|(_, distance)| *distance <= slop)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(row, _)| row)
}

/// Точка (логические точки окна) внутри узла интерфейса; `scale` — масштаб окна.
pub fn node_contains(
    node: &ComputedNode,
    transform: &UiGlobalTransform,
    point: Vec2,
    scale: f32,
) -> bool {
    node.contains_point(*transform, point * scale)
}

/// Игра ушла в фон или потеряла фокус (жест «Домой», звонок, блокировка): всё, что держали пальцы,
/// отпускается. Система обычно присылает отмену касаний, но не обязана; палец, который она так
/// и не отпустила, после возврата ничего не держит — его уже нет в `TouchControls`.
fn release_on_blur(
    mut focus: MessageReader<bevy::window::WindowFocused>,
    mut lifecycle: MessageReader<bevy::window::AppLifecycle>,
    mut controls: ResMut<TouchControls>,
    mut touches: ResMut<Touches>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
) {
    let blurred = focus.read().any(|change| !change.focused);
    let suspended = lifecycle.read().any(|event| {
        matches!(event, bevy::window::AppLifecycle::WillSuspend | bevy::window::AppLifecycle::Suspended)
    });
    if !blurred && !suspended {
        return;
    }
    // Пальцы забываются и в `Touches`: отпускания, которое система не прислала, уже не будет, а палец
    // жеста «Домой» из этого же кадра не должен нажать кнопку в `read_touches` (он идёт следом).
    touches.reset_all();
    for (_, action) in std::mem::take(&mut controls.held) {
        release(action, &mut keys, &mut mouse);
    }
    if std::mem::take(&mut controls.aim_fire) {
        mouse.release(MouseButton::Left);
    }
    controls.fingers.clear();
    controls.tap_fire = 0.0;
    controls.point = None;
    controls.twist = None;
    controls.drive_stick = None;
    controls.aim_stick = None;
}

fn press(
    action: TapAction,
    keys: &mut ButtonInput<KeyCode>,
    mouse: &mut ButtonInput<MouseButton>,
    input: &mut PadInput,
) {
    match action {
        TapAction::Key(key) => keys.press(key),
        TapAction::Mouse(button) => mouse.press(button),
        TapAction::Step(step) => input.weapon_step += step,
    }
}

fn release(
    action: TapAction,
    keys: &mut ButtonInput<KeyCode>,
    mouse: &mut ButtonInput<MouseButton>,
) {
    match action {
        TapAction::Key(key) => keys.release(key),
        TapAction::Mouse(button) => mouse.release(button),
        TapAction::Step(_) => {}
    }
}

/// Точка экрана на плоскости высоты `ground`.
fn ground_point(
    camera: &Camera,
    transform: &GlobalTransform,
    point: Vec2,
    ground: f32,
) -> Option<Vec2> {
    let ray = camera.viewport_to_world(transform, point).ok()?;
    let t = (ground - ray.origin.y) / ray.direction.y;
    (t > 0.0).then(|| {
        let p = ray.origin + *ray.direction * t;
        Vec2::new(p.x, p.z)
    })
}

/// Касания этого кадра: пальцы, стики, кнопки, отметка авиаудара.
#[allow(clippy::too_many_arguments)]
fn read_touches(
    touches: Res<Touches>,
    real: Res<Time<Real>>,
    state: Option<Res<State<GameState>>>,
    targeting: Res<Targeting>,
    settings: Res<Settings>,
    window: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &Transform), With<GameCamera>>,
    rover: Query<&Transform, With<Rover>>,
    buttons: Query<(
        &TapAction,
        &ComputedNode,
        &UiGlobalTransform,
        &InheritedVisibility,
        Option<&ComputedStackIndex>,
    )>,
    mut controls: ResMut<TouchControls>,
    (mut tap, mut swipe, mut last, intro, weapon, terrain): (
        ResMut<TouchTap>,
        ResMut<TouchSwipe>,
        ResMut<LastTouch>,
        Option<Res<crate::intro::Intro>>,
        Res<CurrentWeapon>,
        Res<Terrain>,
    ),
    mut device: ResMut<InputDevice>,
    mut input: ResMut<PadInput>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
) {
    tap.0 = None;
    swipe.0 = 0;
    let Ok(window) = window.single() else {
        return;
    };
    let now = real.elapsed_secs_f64();
    let layout = Layout::new(window.size(), &settings);
    let playing = state
        .as_ref()
        .is_some_and(|s| *s.get() == GameState::Playing);
    let controls = &mut *controls;
    let pointing = points_at(*weapon);

    if touches.any_just_pressed() {
        last.0 = Some(now);
        if *device != InputDevice::Touch {
            info!("Управление: касания");
            *device = InputDevice::Touch;
        }
    }
    if touches.iter().next().is_some() {
        last.0 = Some(now);
    }

    // Новые пальцы.
    for touch in touches.iter_just_pressed() {
        let point = touch.position();
        let button = button_at(point, window.scale_factor(), &buttons);
        // Касание, которое обрывает вступление, больше ничего не нажимает.
        let driving = controls
            .fingers
            .values()
            .any(|finger| finger.control == Control::Drive);
        let control = if intro.as_ref().is_some_and(|intro| intro.active()) {
            Control::Spent
        } else {
            hit(
                point,
                &layout,
                button,
                playing,
                targeting.open,
                pointing,
                driving,
            )
        };
        let base = match control {
            Control::Drive => layout.stick_base(point, layout.drive_radius()),
            Control::Aim => layout.stick_base(point, layout.aim_radius()),
            _ => point,
        };
        if let Control::Button(action) = control {
            press(action, &mut keys, &mut mouse, &mut input);
            controls.held.push((touch.id(), action));
        }
        controls.fingers.insert(
            touch.id(),
            Finger {
                control,
                start: point,
                started: now,
                base,
                moved: false,
            },
        );
    }

    // Отпущенные пальцы: кнопки отпускаются, короткое касание — выстрел или `TouchTap`.
    let ended: Vec<(u64, Vec2, bool)> = touches
        .iter_just_released()
        .map(|t| (t.id(), t.position(), true))
        .chain(
            touches
                .iter_just_canceled()
                .map(|t| (t.id(), t.position(), false)),
        )
        .collect();
    for (id, point, released) in ended {
        controls.held.retain(|&(finger, action)| {
            if finger == id {
                release(action, &mut keys, &mut mouse);
            }
            finger != id
        });
        let Some(finger) = controls.fingers.remove(&id) else {
            continue;
        };
        let short = released && now - finger.started <= f64::from(TAP_SECONDS) && !finger.moved;
        match finger.control {
            Control::Aim if short && playing => {
                input.snap = true;
                controls.tap_fire = TAP_FIRE_SECONDS;
            }
            // Короткое касание точки: огонь держится ещё немного, если оружие перезаряжалось.
            Control::Point if short && playing => controls.tap_fire = TAP_FIRE_SECONDS,
            Control::Free if short => tap.0 = Some(point),
            Control::Free if released => {
                let shift = point - finger.start;
                if shift.x.abs() >= SWIPE * layout.unit && shift.x.abs() > shift.y.abs() * 1.5 {
                    swipe.0 = shift.x.signum() as i32;
                }
            }
            _ => {}
        }
    }

    // Пальцы, которые остались: стики и отметка.
    let axes = camera
        .iter()
        .next()
        .map(|(_, transform)| screen_axes(transform))
        .unwrap_or((Vec2::X, Vec2::NEG_Y));
    let mut aim_fire = false;
    let mut point_fire = false;
    let mut strike_fingers: Vec<(Vec2, Vec2)> = Vec::new();
    controls.drive_stick = None;
    controls.aim_stick = None;
    let ids: Vec<u64> = controls.fingers.keys().copied().collect();
    for id in ids {
        let Some(touch) = touches.get_pressed(id) else {
            continue;
        };
        let point = touch.position();
        let finger = controls.fingers.get_mut(&id).expect("палец есть в списке");
        if point.distance(finger.start) > TAP_SLOP * layout.aim_radius() {
            finger.moved = true;
        }
        // Наведение авиаудара открылось, пока палец держал стик прицела: дальше палец ведёт отметку.
        if targeting.open && matches!(finger.control, Control::Aim | Control::Point) {
            finger.control = Control::Strike;
        }
        match finger.control {
            Control::Drive if playing => {
                let radius = layout.drive_radius();
                finger.base = follow_base(finger.base, point, radius);
                let tilt = deadzone(stick_tilt(finger.base, point, radius), DRIVE_DEADZONE);
                input.drive_world = to_world(tilt, axes);
                controls.drive_stick = Some((
                    finger.base,
                    finger.base + (point - finger.base).clamp_length_max(radius),
                ));
            }
            Control::Aim if playing => {
                let radius = layout.aim_radius();
                let tilt = stick_tilt(finger.base, point, radius);
                if tilt.length() > AIM_DEADZONE {
                    let dir = tilt.normalize();
                    input.aim = dir;
                    input.aim_world = to_world(dir, axes);
                    input.aim_range = Some(range_fraction(tilt.length()));
                    aim_fire = true;
                }
                controls.aim_stick = Some((
                    finger.base,
                    finger.base + (point - finger.base).clamp_length_max(radius),
                ));
            }
            Control::Point if playing => {
                point_fire = true;
                if let Some((camera, transform)) = camera.iter().next() {
                    let transform = GlobalTransform::from(*transform);
                    if let Ok(ray) = camera.viewport_to_world(&transform, point)
                        && let Some(hit) =
                            crate::aim::raycast_terrain(&terrain, ray.origin, *ray.direction)
                    {
                        controls.point = Some(Vec2::new(hit.x, hit.z));
                    }
                }
            }
            Control::Strike if targeting.open => {
                strike_fingers.push((touch.previous_position(), point))
            }
            _ => {}
        }
    }

    // Огонь: оттяжка стика прицела или короткое касание держат левую кнопку мыши.
    controls.tap_fire = (controls.tap_fire - real.delta_secs()).max(0.0);
    // Точка попадания: пока палец на экране и пока держится огонь короткого касания.
    if !point_fire && controls.tap_fire <= 0.0 {
        controls.point = None;
    }
    if !targeting.open {
        input.point_world = controls.point;
    }
    let fire = playing && !targeting.open && (aim_fire || point_fire || controls.tap_fire > 0.0);
    let was_firing = controls.aim_fire;
    if fire && !was_firing {
        mouse.press(MouseButton::Left);
    } else if !fire && was_firing {
        mouse.release(MouseButton::Left);
    }
    controls.aim_fire = fire;

    // Отметка авиаудара: один палец двигает её на свой сдвиг по земле, два — поворачивают серию.
    match strike_fingers.as_slice() {
        [(previous, now_point)] => {
            controls.twist = None;
            if let (Some((camera, transform)), Some(rover)) =
                (camera.iter().next(), rover.iter().next())
            {
                let transform = GlobalTransform::from(*transform);
                let ground = rover.translation.y;
                if let (Some(from), Some(to)) = (
                    ground_point(camera, &transform, *previous, ground),
                    ground_point(camera, &transform, *now_point, ground),
                ) {
                    input.cursor_shift += to - from;
                }
            }
        }
        [(_, a), (_, b), ..] => {
            let angle = (*b - *a).to_angle();
            let twist = controls.twist.get_or_insert(angle);
            let turned = crate::stick_aim::angle_diff(angle, *twist);
            if turned.abs() >= TWIST_STEP {
                // Экран смотрит сверху с осью Y вниз: поворот пальцев по часовой — поворот серии по часовой.
                let step = turned.signum() as i32;
                input.weapon_step -= step;
                *twist += step as f32 * TWIST_STEP;
            }
        }
        [] => controls.twist = None,
    }
}

/// Экранные элементы управления.
#[derive(Component)]
struct TouchOverlay;

/// Группа кнопок наведения авиаудара.
#[derive(Component)]
struct StrikeButtons;

/// Кнопка паузы.
#[derive(Component)]
struct PauseButton;

/// Часть стика на экране.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum StickPart {
    DriveBase,
    DriveKnob,
    AimBase,
    AimKnob,
}

/// Подпись кнопки наведения: размер шрифта задаётся по месту.
#[derive(Component)]
struct ButtonLabel(f32);

/// Цвета элементов управления (без прозрачности: её задают настройки).
const DRIVE_COLOR: Color = Color::srgb(0.17, 0.70, 0.79);
const AIM_COLOR: Color = Color::srgb(0.94, 0.42, 0.23);
const STRIKE_COLOR: Color = Color::srgb(0.65, 0.49, 1.0);
const HUD_COLOR: Color = Color::srgb(0.87, 0.90, 0.93);

fn circle(color: Color) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            border: UiRect::all(Val::Px(2.0)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BackgroundColor(color),
        BorderColor::all(color),
    )
}

fn label(text: &str, size: f32) -> impl Bundle {
    (
        ButtonLabel(size),
        Text::new(text),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(HUD_COLOR),
    )
}

fn round_button(action: TapAction, text: &str, size: f32, edge: Color) -> impl Bundle {
    (
        action,
        Node {
            position_type: PositionType::Absolute,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(2.0)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BackgroundColor(Color::srgba(0.04, 0.06, 0.08, 0.45)),
        BorderColor::all(edge),
        children![label(text, size)],
    )
}

fn spawn_overlay(mut commands: Commands) {
    commands
        .spawn((
            TouchOverlay,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            // Над интерфейсом забега, под занавесом перехода.
            GlobalZIndex(50),
            Visibility::Hidden,
            Pickable::IGNORE,
        ))
        .with_children(|overlay| {
            for part in [
                StickPart::DriveBase,
                StickPart::DriveKnob,
                StickPart::AimBase,
                StickPart::AimKnob,
            ] {
                let color = match part {
                    StickPart::DriveBase | StickPart::DriveKnob => DRIVE_COLOR,
                    StickPart::AimBase | StickPart::AimKnob => AIM_COLOR,
                };
                overlay.spawn((part, circle(color), Pickable::IGNORE));
            }
            overlay.spawn((
                PauseButton,
                round_button(TapAction::Key(KeyCode::Escape), "II", 18.0, HUD_COLOR),
                Pickable::IGNORE,
            ));
            overlay
                .spawn((
                    StrikeButtons,
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    Visibility::Hidden,
                    Pickable::IGNORE,
                ))
                .with_children(|strike| {
                    strike.spawn((
                        StrikeButton::TurnLeft,
                        round_button(TapAction::Step(-1), "<", 22.0, STRIKE_COLOR),
                    ));
                    strike.spawn((
                        StrikeButton::TurnRight,
                        round_button(TapAction::Step(1), ">", 22.0, STRIKE_COLOR),
                    ));
                    strike.spawn((
                        StrikeButton::Strike,
                        round_button(
                            TapAction::Mouse(MouseButton::Left),
                            "STRIKE",
                            16.0,
                            AIM_COLOR,
                        ),
                    ));
                    strike.spawn((
                        StrikeButton::Cancel,
                        round_button(
                            TapAction::Mouse(MouseButton::Right),
                            "CANCEL",
                            14.0,
                            HUD_COLOR,
                        ),
                    ));
                });
        });
}

/// Кнопка наведения авиаудара.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum StrikeButton {
    TurnLeft,
    TurnRight,
    Strike,
    Cancel,
}

/// Ставит узел в точку `center` (логические точки) размером `size`, с учётом масштаба интерфейса.
fn place(node: &mut Node, center: Vec2, size: Vec2, ui_scale: f32) {
    let s = ui_scale.max(f32::EPSILON);
    let (left, top) = ((center.x - size.x * 0.5) / s, (center.y - size.y * 0.5) / s);
    let (width, height) = (size.x / s, size.y / s);
    if node.left != Val::Px(left) {
        node.left = Val::Px(left);
    }
    if node.top != Val::Px(top) {
        node.top = Val::Px(top);
    }
    if node.width != Val::Px(width) {
        node.width = Val::Px(width);
    }
    if node.height != Val::Px(height) {
        node.height = Val::Px(height);
    }
}

/// Показывает элементы управления в забеге при касаниях и ставит их по раскладке.
#[allow(clippy::too_many_arguments)]
fn show_overlay(
    device: Res<InputDevice>,
    state: Res<State<GameState>>,
    targeting: Res<Targeting>,
    settings: Res<Settings>,
    ui_scale: Res<UiScale>,
    controls: Res<TouchControls>,
    weapon: Res<CurrentWeapon>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut overlay: Query<&mut Visibility, (With<TouchOverlay>, Without<StrikeButtons>)>,
    mut strike_group: Query<&mut Visibility, (With<StrikeButtons>, Without<TouchOverlay>)>,
    mut sticks: Query<
        (
            &StickPart,
            &mut Node,
            &mut BackgroundColor,
            &mut BorderColor,
            &mut Visibility,
        ),
        (Without<TouchOverlay>, Without<StrikeButtons>),
    >,
    mut pause: Query<
        (&mut Node, &mut Visibility),
        (
            With<PauseButton>,
            Without<StickPart>,
            Without<TouchOverlay>,
            Without<StrikeButtons>,
        ),
    >,
    mut strike: Query<(&StrikeButton, &mut Node), (Without<StickPart>, Without<PauseButton>)>,
    mut labels: Query<(&ButtonLabel, &mut TextFont)>,
) {
    let shown = *device == InputDevice::Touch && *state.get() == GameState::Playing;
    let wanted = if shown {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut visibility in &mut overlay {
        visibility.set_if_neq(wanted);
    }
    if !shown {
        return;
    }
    let Ok(window) = window.single() else {
        return;
    };
    let layout = Layout::new(window.size(), &settings);
    let ui = ui_scale.0;
    let alpha = f32::from(settings.touch_opacity.clamp(10, 100)) / 100.0;

    let strike_shown = if targeting.open {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut visibility in &mut strike_group {
        visibility.set_if_neq(strike_shown);
    }

    for (part, mut node, mut background, mut border, mut visibility) in &mut sticks {
        let (stick, home, radius, color) = match part {
            StickPart::DriveBase | StickPart::DriveKnob => (
                controls.drive_stick,
                layout.drive_home(),
                layout.drive_radius(),
                DRIVE_COLOR,
            ),
            StickPart::AimBase | StickPart::AimKnob => (
                controls.aim_stick,
                layout.aim_home(),
                layout.aim_radius(),
                AIM_COLOR,
            ),
        };
        let active = stick.is_some();
        let (base, knob) = stick.unwrap_or((home, home));
        let knob_part = matches!(part, StickPart::DriveKnob | StickPart::AimKnob);
        let (center, size) = if knob_part {
            (knob, Vec2::splat(layout.dp(KNOB_RADIUS) * 2.0))
        } else {
            (base, Vec2::splat(radius * 2.0))
        };
        place(&mut node, center, size, ui);
        let fill = if knob_part {
            color.with_alpha(alpha * if active { 0.95 } else { 0.6 })
        } else {
            Color::srgba(0.04, 0.06, 0.08, 0.35 * alpha)
        };
        background.set_if_neq(BackgroundColor(fill));
        let edge = BorderColor::all(color.with_alpha(alpha * 0.8));
        if *border != edge {
            *border = edge;
        }
        // В наведении авиаудара стиков нет; у мортиры и ракет нет стика прицела: бьют пальцем.
        let aim_part = matches!(part, StickPart::AimBase | StickPart::AimKnob);
        let pointing = aim_part && points_at(*weapon) && controls.aim_stick.is_none();
        visibility.set_if_neq(if targeting.open || pointing {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
    }

    let pause_size = layout.dp(PAUSE_SIZE);
    for (mut node, mut visibility) in &mut pause {
        let center = Vec2::new(
            layout.right() - pause_size * 0.5,
            layout.top() + pause_size * 0.5,
        );
        place(&mut node, center, Vec2::splat(pause_size), ui);
        visibility.set_if_neq(if targeting.open {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
    }

    let aim = layout.aim_home();
    let turn = layout.dp(TURN_SIZE);
    let strike_size = layout.dp(STRIKE_SIZE);
    let cancel = Vec2::new(layout.dp(CANCEL_SIZE.0), layout.dp(CANCEL_SIZE.1));
    for (button, mut node) in &mut strike {
        let (center, size) = match button {
            StrikeButton::Strike => (aim, Vec2::splat(strike_size)),
            StrikeButton::TurnLeft => (
                aim + Vec2::new(-strike_size * 0.95, -strike_size * 0.75),
                Vec2::splat(turn),
            ),
            StrikeButton::TurnRight => (
                aim + Vec2::new(-strike_size * 0.15, -strike_size * 1.15),
                Vec2::splat(turn),
            ),
            StrikeButton::Cancel => (
                Vec2::new(
                    layout.right() - cancel.x * 0.5,
                    layout.top() + cancel.y * 0.5,
                ),
                cancel,
            ),
        };
        place(&mut node, center, size, ui);
    }

    // Шрифт подписей — в dp, а интерфейс масштабирует его ещё раз.
    for (label, mut font) in &mut labels {
        let size = FontSize::Px(label.0 * layout.unit * layout.scale / ui.max(f32::EPSILON));
        if font.font_size != size {
            font.font_size = size;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone() -> Layout {
        Layout {
            size: Vec2::new(800.0, 360.0),
            unit: 1.0,
            scale: 1.0,
            inset: Vec4::ZERO,
        }
    }

    #[test]
    fn кнопка_важнее_области_а_середина_ничья() {
        let layout = phone();
        let pause = Some(TapAction::Key(KeyCode::Escape));
        assert_eq!(
            hit(
                Vec2::new(100.0, 300.0),
                &layout,
                pause,
                true,
                false,
                false,
                false
            ),
            Control::Button(pause.unwrap())
        );
        assert_eq!(
            hit(
                Vec2::new(100.0, 300.0),
                &layout,
                None,
                true,
                false,
                false,
                false
            ),
            Control::Drive
        );
        assert_eq!(
            hit(
                Vec2::new(700.0, 300.0),
                &layout,
                None,
                true,
                false,
                false,
                false
            ),
            Control::Aim
        );
        assert_eq!(
            hit(
                Vec2::new(400.0, 300.0),
                &layout,
                None,
                true,
                false,
                false,
                false
            ),
            Control::Free
        );
        // Верхняя полоса — интерфейс.
        assert_eq!(
            hit(
                Vec2::new(100.0, 40.0),
                &layout,
                None,
                true,
                false,
                false,
                false
            ),
            Control::Free
        );
        // Вне забега стиков нет, в наведении авиаудара палец ведёт отметку.
        assert_eq!(
            hit(
                Vec2::new(100.0, 300.0),
                &layout,
                None,
                false,
                false,
                false,
                false
            ),
            Control::Free
        );
        assert_eq!(
            hit(
                Vec2::new(100.0, 300.0),
                &layout,
                None,
                true,
                true,
                false,
                false
            ),
            Control::Strike
        );
    }

    #[test]
    fn у_мортиры_и_ракет_палец_указывает_точку() {
        let layout = phone();
        // Правая область, середина и верх — точка попадания.
        for at in [(700.0, 300.0), (400.0, 300.0), (600.0, 40.0)] {
            assert_eq!(
                hit(Vec2::from(at), &layout, None, true, false, true, false),
                Control::Point
            );
        }
        // Первый палец слева — стик езды, второй слева при взятом стике — точка.
        let left = Vec2::new(100.0, 300.0);
        assert_eq!(
            hit(left, &layout, None, true, false, true, false),
            Control::Drive
        );
        assert_eq!(
            hit(left, &layout, None, true, false, true, true),
            Control::Point
        );
        // Кнопки и наведение авиаудара важнее.
        let pause = Some(TapAction::Key(KeyCode::Escape));
        assert_eq!(
            hit(left, &layout, pause, true, false, true, true),
            Control::Button(pause.unwrap())
        );
        assert_eq!(
            hit(left, &layout, None, true, true, true, true),
            Control::Strike
        );
        assert!(points_at(CurrentWeapon::Mortar) && points_at(CurrentWeapon::HomingRockets));
        assert!(!points_at(CurrentWeapon::MachineGun) && !points_at(CurrentWeapon::Laser));
    }

    #[test]
    fn стик_езды_едет_за_пальцем() {
        let base = Vec2::new(100.0, 300.0);
        // Внутри радиуса центр стоит.
        assert_eq!(follow_base(base, Vec2::new(130.0, 300.0), 60.0), base);
        // Палец ушёл на 100 вправо: центр подтянулся на 40.
        let moved = follow_base(base, Vec2::new(200.0, 300.0), 60.0);
        assert!((moved - Vec2::new(140.0, 300.0)).length() < 1e-4, "{moved}");
    }

    #[test]
    fn наклон_вверх_по_экрану_это_плюс_y() {
        let tilt = stick_tilt(Vec2::new(100.0, 300.0), Vec2::new(100.0, 270.0), 60.0);
        assert!((tilt - Vec2::new(0.0, 0.5)).length() < 1e-5, "{tilt}");
        let full = stick_tilt(Vec2::new(100.0, 300.0), Vec2::new(300.0, 300.0), 60.0);
        assert!((full - Vec2::X).length() < 1e-5, "{full}");
    }

    #[test]
    fn дальность_мортиры_по_оттяжке() {
        assert_eq!(range_fraction(AIM_DEADZONE), 0.0);
        assert_eq!(range_fraction(1.0), 1.0);
        assert!(range_fraction(0.4) < range_fraction(0.8));
    }

    #[test]
    fn стик_не_вылезает_за_экран() {
        let layout = phone();
        let base = layout.stick_base(Vec2::new(5.0, 355.0), 60.0);
        assert!(
            base.x >= MARGIN + 60.0 && base.y <= 360.0 - MARGIN - 60.0,
            "{base}"
        );
    }
}
