//! Разрешение кадра ниже разрешения экрана: на телефоне рисовать каждый физический пиксель
//! (плотность 2,5–3,5 пикселя на точку) мобильной видеокарте не по силам.
//!
//! Игровой мир о масштабе не знает: окно, камера, интерфейс и касания живут в полном размере.
//! В рендер-потоке, сразу после передачи кадра, уменьшаются размер цепочки кадров окна, целевые
//! текстуры камер, нацеленных на окно, и их области вывода, в том же отношении — область вывода
//! интерфейса. Проекции от размера не зависят, поэтому картинка та же, только реже по пикселям.
//! Растягивает кадр на экран сама система: на Android цепочка кадров Vulkan меньше окна
//! масштабируется компоновщиком бесплатно.
//!
//! Плотность кадра — пикселей на точку экрана (dp), по ступени качества; на компьютере масштаба нет.

use bevy::camera::NormalizedRenderTarget;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::view::window::{ExtractedWindow, create_surfaces};
use bevy::render::view::ExtractedView;
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::ui_render::UiViewTarget;
use bevy::window::PrimaryWindow;

use crate::quality::Quality;

/// Доля стороны кадра от стороны экрана, 0 < k ≤ 1.
#[derive(Resource, ExtractResource, Clone, Copy, Debug, PartialEq)]
#[extract_app(RenderApp)]
pub struct RenderScale(pub f32);

impl Default for RenderScale {
    fn default() -> Self {
        RenderScale(1.0)
    }
}

/// Сколько пикселей кадра на точку экрана на этой ступени; `None` — экран как есть.
fn density(quality: Quality) -> Option<f32> {
    if !cfg!(target_os = "android") {
        return None;
    }
    if let Some(density) = density_override() {
        return (density > 0.0).then_some(density);
    }
    Some(match quality {
        Quality::Low => 1.25,
        Quality::Medium => 1.5,
        Quality::High => 2.0,
    })
}

/// Плотность из свойства Android `debug.spaceguard.density` для замеров без пересборки:
/// `adb shell setprop debug.spaceguard.density 0` — полный экран, `2` — два пикселя на точку.
/// Читается при запуске игры.
fn density_override() -> Option<f32> {
    #[cfg(target_os = "android")]
    {
        use std::sync::OnceLock;
        static VALUE: OnceLock<Option<f32>> = OnceLock::new();
        *VALUE.get_or_init(|| {
            let text = crate::android::system_property(c"debug.spaceguard.density")?;
            info!("Плотность кадра из debug.spaceguard.density: {text}");
            text.trim().parse().ok()
        })
    }
    #[cfg(not(target_os = "android"))]
    {
        None
    }
}

pub struct RenderScalePlugin;

impl Plugin for RenderScalePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderScale>()
            .add_plugins(ExtractResourcePlugin::<RenderScale>::default())
            .add_systems(Update, choose_scale.in_set(crate::frame::GameSet::Presentation));
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            // После набора `ExtractCommands`: в нём применяются команды передачи кадра, которые
            // вставляют свежие компоненты камер и новых окон с полными размерами.
            render_app.add_systems(
                Render,
                shrink_views
                    .after(RenderSystems::ExtractCommands)
                    .before(RenderSystems::PrepareAssets)
                    .before(create_surfaces),
            );
        }
    }
}

/// Масштаб по ступени качества и плотности экрана.
fn choose_scale(
    quality: Res<Quality>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut scale: ResMut<RenderScale>,
) {
    let wanted = density(*quality).map_or(1.0, |d| (d / window.scale_factor()).clamp(0.25, 1.0));
    if (scale.0 - wanted).abs() > 1e-4 {
        info!("Разрешение кадра: {:.0} % стороны экрана", wanted * 100.0);
        scale.0 = wanted;
    }
}

/// Сторона в пикселях после масштаба.
fn scaled(size: UVec2, k: f32) -> UVec2 {
    (size.as_vec2() * k).round().as_uvec2().max(UVec2::ONE)
}

/// Полный и уменьшенный размер окна в прошлом кадре.
#[derive(Clone, Copy)]
struct WindowSizes {
    full: UVec2,
    scaled: UVec2,
}

/// Уменьшает окно, камеры на окне и их интерфейс. Передача кадра каждый раз пишет полные размеры
/// (окну — только когда его размер изменился), поэтому полный размер окна помнится здесь, а флаг
/// изменения размера пересчитывается, чтобы цепочка кадров не пересоздавалась каждый кадр.
fn shrink_views(
    scale: Option<Res<RenderScale>>,
    mut windows: Query<(Entity, &mut ExtractedWindow)>,
    mut cameras: Query<(Entity, &mut ExtractedCamera, &mut ExtractedView)>,
    mut ui_views: Query<(&UiViewTarget, &mut ExtractedView), Without<ExtractedCamera>>,
    mut sizes: Local<HashMap<Entity, WindowSizes>>,
) {
    let k = scale.map_or(1.0, |s| s.0);
    for (entity, mut window) in &mut windows {
        let current = UVec2::new(window.physical_width, window.physical_height);
        let before = sizes.get(&entity).copied();
        let full = match before {
            Some(sizes) if !window.size_changed => sizes.full,
            _ => current,
        };
        let size = scaled(full, k);
        window.physical_width = size.x;
        window.physical_height = size.y;
        // Поверхность настроена на размер, оставленный здесь в прошлом кадре: перенастройка нужна,
        // только когда он меняется. В первом кадре поверхность ещё создаётся.
        window.size_changed = before.is_none_or(|b| b.scaled != size);
        if before.is_none_or(|b| b.scaled != size || b.full != full) {
            info!("Разрешение кадра: окно {}×{}, кадр {}×{}", full.x, full.y, size.x, size.y);
        }
        sizes.insert(entity, WindowSizes { full, scaled: size });
    }
    if k >= 1.0 {
        return;
    }
    let mut shrunk = HashMap::new();
    for (entity, mut camera, mut view) in &mut cameras {
        if !matches!(camera.target, Some(NormalizedRenderTarget::Window(_))) {
            continue;
        }
        camera.physical_target_size = camera.physical_target_size.map(|s| scaled(s, k));
        camera.physical_viewport_size = camera.physical_viewport_size.map(|s| scaled(s, k));
        if let Some(viewport) = camera.viewport.as_mut() {
            viewport.physical_position = (viewport.physical_position.as_vec2() * k).round().as_uvec2();
            viewport.physical_size = scaled(viewport.physical_size, k);
        }
        let origin = (view.viewport.xy().as_vec2() * k).round().as_uvec2();
        let size = scaled(view.viewport.zw(), k);
        view.viewport = UVec4::new(origin.x, origin.y, size.x, size.y);
        shrunk.insert(entity, view.viewport);
    }
    for (target, mut view) in &mut ui_views {
        if let Some(viewport) = shrunk.get(&target.0) {
            view.viewport = *viewport;
        }
    }
}
