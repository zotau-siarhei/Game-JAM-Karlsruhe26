//! Игровой генератор случайных чисел: волны, враги, ящики, поезда, жители.
//!
//! Отделён от звукового [`Dice`]: состав волны не зависит от того, сколько звуков успело
//! сыграть. Пересеивается от сида мира при каждой его перестройке, поэтому забег с тем же сидом
//! получает ту же последовательность случайных чисел.

use std::ops::{Deref, DerefMut};

use bevy::prelude::*;

use crate::frame::GameSet;
use crate::run::WorldRebuilt;
use crate::sound_list::Dice;

/// Подключает игровой генератор и его пересев.
pub struct RngPlugin;

impl Plugin for RngPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameRng>()
            .add_systems(Update, reseed.in_set(GameSet::Reset));
    }
}

/// Игровой генератор. Методы — как у [`Dice`]: `below`, `range`.
#[derive(Resource)]
pub struct GameRng(Dice);

impl GameRng {
    /// Генератор, засеянный сидом мира.
    pub fn from_seed(seed: u32) -> Self {
        Self(Dice::seeded(u64::from(seed)))
    }
}

impl Default for GameRng {
    fn default() -> Self {
        Self::from_seed(0)
    }
}

impl Deref for GameRng {
    type Target = Dice;

    fn deref(&self) -> &Dice {
        &self.0
    }
}

impl DerefMut for GameRng {
    fn deref_mut(&mut self) -> &mut Dice {
        &mut self.0
    }
}

/// Мир перестроен: генератор начинает последовательность нового сида.
fn reseed(mut rebuilt: MessageReader<WorldRebuilt>, mut rng: ResMut<GameRng>) {
    if let Some(world) = rebuilt.read().last() {
        *rng = GameRng::from_seed(world.seed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_gives_the_same_sequence() {
        let mut first = GameRng::from_seed(42);
        let mut second = GameRng::from_seed(42);
        for _ in 0..20 {
            assert_eq!(first.below(1000), second.below(1000));
        }
    }

    #[test]
    fn different_seeds_give_different_sequences() {
        let mut first = GameRng::from_seed(1);
        let mut second = GameRng::from_seed(2);
        let same = (0..20).filter(|_| first.below(1000) == second.below(1000)).count();
        assert!(same < 5, "совпало {same} из 20");
    }

    #[test]
    fn zero_seed_is_not_stuck() {
        let mut rng = GameRng::from_seed(0);
        let values: Vec<usize> = (0..10).map(|_| rng.below(1000)).collect();
        assert!(values.iter().any(|value| *value != values[0]), "{values:?}");
    }
}
