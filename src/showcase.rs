//! Облёт разрушений после гибели: за экраном рекордов камера летит над картой и показывает,
//! что осталось от посёлков.
//!
//! Во время забега запоминаются места рухнувших домов и мощных взрывов (не ближе
//! [`MIN_SPACING`] друг к другу). На экране конца забега фокус камеры ([`Focus`]) стоит у места
//! гибели [`START_PAUSE`] секунд, потом летит по этим местам от ближайшего к ближайшему и по кругу.
//! Мест нет — медленно кружит над местом гибели. Идёт по реальному времени.

use bevy::prelude::*;

use crate::buildings::BuildingDestroyed;
use crate::camera::Focus;
use crate::explosion::Explosion;
use crate::frame::GameSet;
use crate::run::{GameState, WorldRebuilt};
use crate::terrain::Terrain;

/// Взрывы мощнее этого — достопримечательность: большой кратер.
const BIG_BLAST: f32 = 1.0;
/// Места ближе этого считаются одним, м.
const MIN_SPACING: f32 = 14.0;
/// Сколько мест помнится: последние.
const MAX_SPOTS: usize = 40;
/// Сколько секунд камера стоит у места гибели.
const START_PAUSE: f32 = 1.5;
/// Скорость облёта, м/с.
const SPEED: f32 = 9.0;
/// Место считается пройденным ближе этого, м.
const REACHED: f32 = 2.0;
/// Круг над местом гибели, если разрушений нет: радиус, м, и угловая скорость, рад/с.
const ORBIT: (f32, f32) = (12.0, 0.25);
/// Высота фокуса над рельефом, м: как у лунохода.
const LIFT: f32 = 0.6;

/// Подключает облёт разрушений.
pub struct ShowcasePlugin;

impl Plugin for ShowcasePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Spots>()
            .init_resource::<Tour>()
            .add_systems(OnEnter(GameState::GameOver), start_tour)
            .add_systems(
                Update,
                (
                    forget_spots.in_set(GameSet::Reset),
                    remember_spots.in_set(GameSet::Presentation),
                    fly_tour
                        .run_if(in_state(GameState::GameOver))
                        .after(GameSet::Cleanup)
                        .before(GameSet::Streaming),
                ),
            );
    }
}

/// Места разрушений забега по порядку появления.
#[derive(Resource, Default)]
struct Spots(Vec<Vec3>);

impl Spots {
    fn add(&mut self, p: Vec3) {
        if self.0.iter().any(|s| s.with_y(0.0).distance(p.with_y(0.0)) < MIN_SPACING) {
            return;
        }
        self.0.push(p);
        let excess = self.0.len().saturating_sub(MAX_SPOTS);
        self.0.drain(..excess);
    }
}

/// Маршрут облёта.
#[derive(Resource, Default)]
struct Tour {
    /// Места в порядке облёта.
    route: Vec<Vec2>,
    next: usize,
    /// Место гибели: центр круга, если мест нет.
    home: Vec2,
    elapsed: f32,
}

fn forget_spots(mut rebuilt: MessageReader<WorldRebuilt>, mut spots: ResMut<Spots>) {
    if rebuilt.read().count() > 0 {
        spots.0.clear();
    }
}

fn remember_spots(
    state: Res<State<GameState>>,
    mut destroyed: MessageReader<BuildingDestroyed>,
    mut explosions: MessageReader<Explosion>,
    mut spots: ResMut<Spots>,
) {
    let playing = *state.get() == GameState::Playing;
    for house in destroyed.read() {
        if playing {
            spots.add(house.position);
        }
    }
    for blast in explosions.read() {
        if playing && blast.power >= BIG_BLAST {
            spots.add(blast.position);
        }
    }
}

/// Маршрут от места гибели: каждый раз к ближайшему ещё не пройденному месту.
fn route_from(home: Vec2, spots: &[Vec2]) -> Vec<Vec2> {
    let mut left = spots.to_vec();
    let mut at = home;
    let mut route = Vec::with_capacity(left.len());
    while !left.is_empty() {
        let (i, _) = left
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.distance_squared(at)))
            .fold((0, f32::MAX), |best, (i, d)| if d < best.1 { (i, d) } else { best });
        at = left.swap_remove(i);
        route.push(at);
    }
    route
}

fn start_tour(focus: Res<Focus>, spots: Res<Spots>, mut tour: ResMut<Tour>) {
    let home = Vec2::new(focus.0.x, focus.0.z);
    let flat: Vec<Vec2> = spots.0.iter().map(|p| Vec2::new(p.x, p.z)).collect();
    *tour = Tour {
        route: route_from(home, &flat),
        next: 0,
        home,
        elapsed: 0.0,
    };
}

fn fly_tour(real: Res<Time<Real>>, terrain: Res<Terrain>, mut tour: ResMut<Tour>, mut focus: ResMut<Focus>) {
    let dt = real.delta_secs();
    tour.elapsed += dt;
    if tour.elapsed < START_PAUSE {
        return;
    }
    let at = Vec2::new(focus.0.x, focus.0.z);
    let next = if tour.route.is_empty() {
        let angle = (tour.elapsed - START_PAUSE) * ORBIT.1;
        tour.home + Vec2::from_angle(angle) * ORBIT.0
    } else {
        let target = tour.route[tour.next % tour.route.len()];
        if at.distance(target) < REACHED {
            tour.next += 1;
        }
        target
    };
    let moved = at + (next - at).clamp_length_max(SPEED * dt);
    focus.0 = Vec3::new(moved.x, terrain.height_at(moved.x, moved.y) + LIFT, moved.y);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn маршрут_идёт_к_ближайшему() {
        let route = route_from(Vec2::ZERO, &[Vec2::new(50.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(30.0, 0.0)]);
        assert_eq!(route, vec![Vec2::new(10.0, 0.0), Vec2::new(30.0, 0.0), Vec2::new(50.0, 0.0)]);
    }

    #[test]
    fn близкие_места_сливаются() {
        let mut spots = Spots::default();
        spots.add(Vec3::ZERO);
        spots.add(Vec3::new(5.0, 0.0, 0.0));
        spots.add(Vec3::new(20.0, 0.0, 0.0));
        assert_eq!(spots.0.len(), 2);
    }
}
