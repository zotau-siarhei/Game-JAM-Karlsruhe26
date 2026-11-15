//! Порядок кадра: наборы систем `Update`, в которые каждый плагин ставит свои системы.
//!
//! Наборы идут цепочкой, между ними применяются отложенные команды:
//!
//! ```text
//! Screens → Reset → Input → Simulation → Damage → ApplyDamage → Death → Cleanup → Streaming → Presentation
//! ```
//!
//! Наборы от `Input` до `Cleanup` — игровая симуляция: они работают только в
//! `GameState::Playing`. В меню, на паузе и на экране конца забега мир стоит, а экраны, сброс,
//! подгрузка мира, интерфейс, эффекты и звук продолжают работать.

use bevy::prelude::*;

use crate::run::GameState;

/// Место системы в кадре.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameSet {
    /// Экраны и переходы забега: меню, пауза, конец забега, рестарт. Работает всегда.
    /// Здесь `RunWorld::reset` перестраивает мир и шлёт `WorldRebuilt`.
    Screens,
    /// Сброс состояния по `WorldRebuilt` и `RunStarted`. Работает всегда, в том же кадре, что
    /// и перестройка мира.
    Reset,
    /// Ввод игрока: езда, выбор оружия, спуск, ремонт. Только в забеге.
    Input,
    /// Игровая логика: враги, волны, снаряды, поезда, жители, таймеры. Только в забеге.
    Simulation,
    /// Взрывы и попадания превращаются в урон: кратеры, `TerrainChanged`, провалы, сообщения
    /// `Damage` от взрывов. Только в забеге.
    Damage,
    /// Одна система `combat::apply_damage`: вычитает здоровье и шлёт `Died`. Сюда ничего не добавлять.
    ApplyDamage,
    /// Последствия гибели: читатели `Died`. Погибшая сущность ещё существует. Только в забеге.
    Death,
    /// Уборка: удаление погибших, улетевших и отставших. Только в забеге.
    Cleanup,
    /// Подгрузка мира вокруг фокуса камеры: участки рельефа и их меши, дома, объекты, пути,
    /// обломки. Работает всегда.
    Streaming,
    /// Камера, интерфейс, эффекты, звук: всё, что только показывает мир. Работает всегда.
    Presentation,
}

/// Задаёт порядок наборов и условие «только в забеге» для симуляции.
pub struct FramePlugin;

impl Plugin for FramePlugin {
    fn build(&self, app: &mut App) {
        use GameSet::*;
        app.configure_sets(
            Update,
            (
                Screens,
                Reset,
                Input,
                Simulation,
                Damage,
                ApplyDamage,
                Death,
                Cleanup,
                Streaming,
                Presentation,
            )
                .chain(),
        );
        for set in [Input, Simulation, Damage, ApplyDamage, Death, Cleanup] {
            app.configure_sets(Update, set.run_if(in_state(GameState::Playing)));
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;

    use super::*;

    /// Кадр игры собирается: в порядке систем и наборов нет циклов, у систем нет конфликтов
    /// параметров. Игра это проверяет только при запуске, а запуск требует окна.
    #[test]
    fn update_schedule_builds_without_cycles() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin).add_plugins(crate::game_plugins);
        let built = app
            .world_mut()
            .try_schedule_scope(Update, |world, schedule| schedule.initialize(world))
            .expect("расписание Update есть");
        if let Err(error) = built {
            panic!("кадр не собирается: {error:?}");
        }
    }
}
