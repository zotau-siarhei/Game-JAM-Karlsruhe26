//! Затемнение: общий чёрный занавес поверх всего и плавное появление экранов.
//!
//! Занавес закрыт при запуске (под ним идёт загрузка, `src/loading.rs`) и закрывается на время
//! перестройки мира (`src/run.rs`): игрок не видит, как мир собирается. Время — реальное, чтобы
//! переходы шли и на паузе.
//!
//! [`Appear`] на корне экрана проявляет фон и текст всех его узлов: доля от 0 до 1 умножает их
//! исходную прозрачность. С отрицательной скоростью экран гаснет.

use bevy::prelude::*;

/// Слой занавеса: выше всех экранов игры.
const CURTAIN_Z: i32 = 1_000_000;
/// Слой того, что рисуется поверх занавеса: экран загрузки.
pub const ABOVE_CURTAIN_Z: i32 = CURTAIN_Z + 1;

/// Плагин затемнения.
pub struct FadePlugin;

impl Plugin for FadePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Curtain::default())
            .add_systems(Startup, spawn_curtain)
            .add_systems(
                Update,
                (move_curtain.in_set(CurtainMove), appear).in_set(crate::frame::GameSet::Presentation),
            );
    }
}

/// Шаг и окраска занавеса за кадр: кто ставит занавес сразу (`Curtain::dim`), стоит до него.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct CurtainMove;

/// Чёрный занавес: непрозрачность сейчас и к какой он идёт.
#[derive(Resource, Debug)]
pub struct Curtain {
    alpha: f32,
    target: f32,
    /// Доля непрозрачности в секунду.
    speed: f32,
}

impl Default for Curtain {
    /// Закрыт: игра начинается с загрузки под ним.
    fn default() -> Self {
        Self {
            alpha: 1.0,
            target: 1.0,
            speed: 1.0,
        }
    }
}

impl Curtain {
    /// Закрыть за `seconds` (полный ход от открытого).
    pub fn close(&mut self, seconds: f32) {
        self.target = 1.0;
        self.speed = 1.0 / seconds.max(0.01);
    }

    /// Притемнить сразу, в этом же кадре, до непрозрачности `alpha`: экран за занавесом виден.
    pub fn dim(&mut self, alpha: f32) {
        self.alpha = alpha;
        self.target = alpha;
    }

    /// Открыть за `seconds` (полный ход от закрытого).
    pub fn open(&mut self, seconds: f32) {
        self.target = 0.0;
        self.speed = 1.0 / seconds.max(0.01);
    }

    /// Занавес полностью закрыт: за ним можно перестраивать мир.
    pub fn is_closed(&self) -> bool {
        self.alpha >= 1.0
    }

    fn step(&mut self, dt: f32) {
        let step = self.speed * dt;
        self.alpha = if self.alpha < self.target {
            (self.alpha + step).min(self.target)
        } else {
            (self.alpha - step).max(self.target)
        };
    }
}

#[derive(Component)]
struct CurtainNode;

fn spawn_curtain(mut commands: Commands) {
    commands.spawn((
        CurtainNode,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            ..default()
        },
        BackgroundColor(Color::BLACK),
        GlobalZIndex(CURTAIN_Z),
    ));
}

/// Двигает занавес и красит его узел. Открытый занавес не рисуется.
fn move_curtain(
    time: Res<Time<Real>>,
    mut curtain: ResMut<Curtain>,
    mut nodes: Query<(&mut BackgroundColor, &mut Visibility), With<CurtainNode>>,
) {
    // Шаг ограничен: долгий кадр (компиляция, загрузка) не проматывает переход целиком.
    curtain.step(time.delta_secs().min(1.0 / 30.0));
    let alpha = curtain.alpha;
    for (mut color, mut visibility) in &mut nodes {
        let wanted = if alpha > 0.0 { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
        if color.0.alpha() != alpha {
            color.0.set_alpha(alpha);
        }
    }
}

/// Плавное появление (или угасание) экрана: доля от 0 до 1 и скорость в долях в секунду.
#[derive(Component, Debug, Clone, Copy)]
pub struct Appear {
    progress: f32,
    speed: f32,
}

impl Appear {
    /// Проявиться из прозрачного за `seconds`.
    pub fn over(seconds: f32) -> Self {
        Self {
            progress: 0.0,
            speed: 1.0 / seconds.max(0.01),
        }
    }

    /// Виден сразу.
    pub fn shown() -> Self {
        Self {
            progress: 1.0,
            speed: 0.0,
        }
    }

    /// Погаснуть за `seconds`.
    pub fn fade_out(&mut self, seconds: f32) {
        self.speed = -1.0 / seconds.max(0.01);
    }

    /// Сдвигает долю; `true`, если она изменилась.
    fn step(&mut self, dt: f32) -> bool {
        let next = (self.progress + self.speed * dt).clamp(0.0, 1.0);
        let changed = next != self.progress;
        self.progress = next;
        changed
    }
}

/// Исходная прозрачность фона и текста узла под [`Appear`].
#[derive(Component, Clone, Copy)]
struct BaseAlpha {
    background: f32,
    text: f32,
}

fn appear(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut roots: Query<(Entity, &mut Appear)>,
    children: Query<&Children>,
    mut nodes: Query<(Option<&mut BackgroundColor>, Option<&mut TextColor>, Option<&BaseAlpha>)>,
) {
    let dt = time.delta_secs().min(1.0 / 30.0);
    for (root, mut appear) in &mut roots {
        let added = appear.is_added();
        if !appear.step(dt) && !added {
            continue;
        }
        let share = appear.progress;
        for entity in std::iter::once(root).chain(children.iter_descendants(root)) {
            let Ok((background, text, base)) = nodes.get_mut(entity) else {
                continue;
            };
            let base = base.copied().unwrap_or_else(|| {
                let base = BaseAlpha {
                    background: background.as_ref().map_or(1.0, |c| c.0.alpha()),
                    text: text.as_ref().map_or(1.0, |c| c.0.alpha()),
                };
                commands.entity(entity).insert(base);
                base
            });
            if let Some(mut background) = background {
                background.0.set_alpha(base.background * share);
            }
            if let Some(mut text) = text {
                text.0.set_alpha(base.text * share);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn занавес_открывается_и_закрывается_за_заданное_время() {
        let mut curtain = Curtain::default();
        assert!(curtain.is_closed());
        curtain.open(0.5);
        curtain.step(0.25);
        assert!((curtain.alpha - 0.5).abs() < 1e-4);
        curtain.step(1.0);
        assert_eq!(curtain.alpha, 0.0);
        curtain.close(0.2);
        curtain.step(0.1);
        assert!(!curtain.is_closed());
        curtain.step(0.1);
        assert!(curtain.is_closed());
    }

    #[test]
    fn экран_проявляется_и_гаснет() {
        let mut appear = Appear::over(0.5);
        assert!(appear.step(0.25));
        assert!((appear.progress - 0.5).abs() < 1e-4);
        appear.step(1.0);
        assert_eq!(appear.progress, 1.0);
        assert!(!appear.step(0.1));
        appear.fade_out(0.3);
        appear.step(1.0);
        assert_eq!(appear.progress, 0.0);
    }
}
