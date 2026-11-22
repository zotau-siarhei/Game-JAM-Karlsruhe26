//! Прицел правым стиком.
//!
//! Стик задаёт только направление прицела от лунохода, дальность берётся от пойманного врага или от
//! оружия: у пулемёта, ракет и лазера — предел дальности, у мортиры и авиаудара — последняя выбранная.
//! Новое направление принимается лишь при уверенном наклоне, который не падает быстро: так отскок
//! пружины при отпускании стика прицел не сдвигает. Враг рядом с линией прицела ловится, и прицел
//! плавно доворачивает к нему. Пока зажат L2, стик двигает точку прицела по земле как курсор.
//!
//! Точка хранится курсом и дальностью от лунохода: при езде она едет вместе с ним, а сглаживание
//! курса идёт по дуге и не проводит точку через луноход.

use bevy::prelude::*;

use crate::aim::{AimPoint, MAX_AIM_RANGE};
use crate::combat::Health;
use crate::difficulty::{AimAssist, RunDifficulty};
use crate::enemy::Enemy;
use crate::frame::GameSet;
use crate::gamepad::{InputDevice, PadInput};
use crate::haptics::Rumble;
use crate::rover::Rover;
use crate::run::{GameState, WorldRebuilt};
use crate::steering;
use crate::terrain::Terrain;
use crate::weapons::CurrentWeapon;

/// Числа настройки прицела стиком.
mod tuning {
    /// Наклон стика после мёртвой зоны, с которого принимается новое направление.
    pub const ACCEPT_TILT: f32 = 0.35;
    /// Наклон, падающий быстрее этого (долей хода в секунду), — стик отпускают: кадр не берётся.
    pub const RELEASE_RATE: f32 = 4.0;
    /// Дальность мортиры и авиаудара, пока игрок её не выбрал, м.
    pub const DEFAULT_RANGE: f32 = 15.0;
    /// Скорость подхода показанного курса к нужному, 1/с.
    pub const HEADING_SMOOTH: f32 = 20.0;
    /// Скорость подхода показанной дальности к нужной, 1/с.
    pub const RANGE_SMOOTH: f32 = 12.0;
    /// Дальность мортиры при наименьшей оттяжке сенсорного стика, м.
    pub const TOUCH_MIN_RANGE: f32 = 5.0;
    /// Ближе этого помощь врага не ловит, м.
    pub const MIN_ASSIST_RANGE: f32 = 3.0;
    // Зона захвата, удержание пойманного врага и замедление курсора над врагом — по сложности
    // забега: `difficulty::Difficulty::aim_assist`.
    /// Поправка оценки на дистанцию: метров отклонения за метр дистанции.
    pub const RANGE_WEIGHT: f32 = 0.05;
    /// Новый враг перехватывает прицел, только если его оценка лучше на столько, м.
    pub const SWITCH_MARGIN: f32 = 1.0;
    /// Скорость курсора при полном наклоне, м/с.
    pub const CURSOR_SPEED: f32 = 30.0;
    /// Степень кривой скорости курсора от наклона: малый наклон — точная доводка.
    pub const CURSOR_CURVE: f32 = 2.0;
    /// Курсор ближе этого к врагу замедляется, м.
    pub const CURSOR_FRICTION_RADIUS: f32 = 2.5;
    /// Толчок слабого мотора при захвате нового врага.
    pub const LOCK_RUMBLE: f32 = 0.25;
    /// Откуда проверяется видимость врага: высота над луноходом, м.
    pub const EYE_HEIGHT: f32 = 1.0;
    /// Куда проверяется видимость: высота над основанием врага, м.
    pub const TARGET_HEIGHT: f32 = 0.6;
}

use tuning::*;

/// Плагин прицела стиком.
pub struct StickAimPlugin;

impl Plugin for StickAimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StickAim>().add_systems(
            Update,
            (
                reset_aim.in_set(GameSet::Reset),
                // После прицела мышью и до стрельбы: в режиме геймпада мышь прицел не трогает.
                aim_with_stick
                    .run_if(in_state(GameState::Playing))
                    .after(crate::aim::update_aim_point)
                    .after(GameSet::Reset)
                    .before(GameSet::Input),
            ),
        );
    }
}

/// Прицел со стика: курс и дальность от лунохода, нужные и показанные.
#[derive(Resource)]
struct StickAim {
    /// Курс задан: до первого кадра забега прицел смотрит по курсу лунохода.
    started: bool,
    /// Нужный курс на плоскости (x, z), рад.
    heading: f32,
    /// Нужная дальность, м.
    range: f32,
    /// Показанный курс: плавно подходит к нужному.
    shown_heading: f32,
    /// Показанная дальность: плавно подходит к нужной.
    shown_range: f32,
    /// Дальность мортиры и авиаудара: выбранная курсором или по пойманному врагу, м.
    manual_range: f32,
    /// Пойманный враг.
    locked: Option<Entity>,
    /// Наклон стика в прошлом кадре.
    last_tilt: f32,
}

impl Default for StickAim {
    fn default() -> Self {
        Self {
            started: false,
            heading: 0.0,
            range: DEFAULT_RANGE,
            shown_heading: 0.0,
            shown_range: DEFAULT_RANGE,
            manual_range: DEFAULT_RANGE,
            locked: None,
            last_tilt: 0.0,
        }
    }
}

fn reset_aim(mut rebuilt: MessageReader<WorldRebuilt>, mut aim: ResMut<StickAim>) {
    if rebuilt.read().last().is_some() {
        *aim = StickAim::default();
    }
}

/// Принимается ли направление стика в этом кадре: наклон уверенный и не падает быстро.
fn accept_heading(tilt: f32, last_tilt: f32, dt: f32) -> bool {
    tilt >= ACCEPT_TILT && last_tilt - tilt <= RELEASE_RATE * dt
}

/// Дальность прицела без пойманного врага.
fn weapon_range(weapon: CurrentWeapon, manual_range: f32) -> f32 {
    match weapon {
        CurrentWeapon::Mortar | CurrentWeapon::Airstrike => manual_range,
        _ => MAX_AIM_RANGE,
    }
}

/// Разность углов `to − from` по кратчайшей дуге.
pub fn angle_diff(to: f32, from: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (to - from + PI).rem_euclid(TAU) - PI
}

/// Враг для помощи в прицеливании: смещения врагов от лунохода, курс `dir` (единичный) и текущая
/// цель. Враг ловится впереди, в пределах дальности и в полосе захвата `assist` вдоль линии
/// прицела; текущий держится в полосе удержания и сменяется, только если другой лучше на
/// [`SWITCH_MARGIN`]. `visible` проверяет прямую видимость, её спрашивают только у прошедших отбор.
fn pick_target(
    dir: Vec2,
    enemies: &[(Entity, Vec2)],
    current: Option<Entity>,
    assist: AimAssist,
    mut visible: impl FnMut(Vec2) -> bool,
) -> Option<(Entity, Vec2)> {
    let mut scored: Vec<(Entity, Vec2, f32)> = enemies
        .iter()
        .filter_map(|&(entity, offset)| {
            let distance = offset.length();
            if offset.dot(dir) <= 0.0 || !(MIN_ASSIST_RANGE..=MAX_AIM_RANGE).contains(&distance) {
                return None;
            }
            let lateral = dir.perp_dot(offset).abs();
            let width = if Some(entity) == current { assist.keep_width } else { assist.catch_width };
            (lateral <= width).then_some((entity, offset, lateral + RANGE_WEIGHT * distance))
        })
        .collect();
    scored.sort_by(|a, b| a.2.total_cmp(&b.2));
    let held = current
        .and_then(|current| scored.iter().find(|(entity, ..)| *entity == current).copied())
        .filter(|(_, offset, _)| visible(*offset));
    let best = scored.into_iter().find(|(_, offset, _)| visible(*offset));
    let chosen = match (held, best) {
        (Some(held), Some(best)) if best.2 + SWITCH_MARGIN < held.2 => Some(best),
        (Some(held), _) => Some(held),
        (None, best) => best,
    };
    chosen.map(|(entity, offset, _)| (entity, offset))
}

/// Ближайший видимый враг в пределах прицела: короткое касание стика прицела.
fn nearest_visible(enemies: &[(Entity, Vec2)], mut visible: impl FnMut(Vec2) -> bool) -> Option<(Entity, Vec2)> {
    let mut near: Vec<(Entity, Vec2)> = enemies
        .iter()
        .copied()
        .filter(|(_, offset)| (MIN_ASSIST_RANGE..=MAX_AIM_RANGE).contains(&offset.length()))
        .collect();
    near.sort_by(|a, b| a.1.length_squared().total_cmp(&b.1.length_squared()));
    near.into_iter().find(|(_, offset)| visible(*offset))
}

/// Шаг курсора за кадр: скорость по кривой от наклона, над врагом медленнее.
fn cursor_step(stick_world: Vec2, dt: f32, near_enemy: bool, assist: AimAssist) -> Vec2 {
    let tilt = stick_world.length().min(1.0);
    if tilt <= 0.0 {
        return Vec2::ZERO;
    }
    let friction = if near_enemy { assist.cursor_friction } else { 1.0 };
    stick_world / stick_world.length() * CURSOR_SPEED * tilt.powf(CURSOR_CURVE) * friction * dt
}

/// Прицел правым стиком: направление, помощь в прицеливании, курсор по L2, сглаживание.
#[allow(clippy::too_many_arguments)]
fn aim_with_stick(
    time: Res<Time>,
    real: Res<Time<Real>>,
    targeting: Res<crate::airstrike::Targeting>,
    device: Res<InputDevice>,
    input: Res<PadInput>,
    weapon: Res<CurrentWeapon>,
    difficulty: Option<Res<RunDifficulty>>,
    terrain: Res<Terrain>,
    mut stick: ResMut<StickAim>,
    mut aim: ResMut<AimPoint>,
    mut rumble: ResMut<Rumble>,
    rover: Single<&Transform, With<Rover>>,
    enemies: Query<(Entity, &Transform, &Health), With<Enemy>>,
    obstacles: crate::obstacles::NearObstacles,
) {
    if !device.stick() {
        return;
    }
    // В наведении авиаудара время замедлено, а курсор идёт по реальному.
    let dt = if targeting.active() { real.delta_secs() } else { time.delta_secs() };
    let assist = difficulty.map(|d| d.0).unwrap_or_default().aim_assist();
    let me = Vec2::new(rover.translation.x, rover.translation.z);
    if !stick.started {
        let forward = Vec2::new(rover.forward().x, rover.forward().z).normalize_or(Vec2::NEG_Y);
        let range = weapon_range(*weapon, stick.manual_range);
        stick.started = true;
        stick.heading = forward.to_angle();
        stick.shown_heading = stick.heading;
        stick.range = range;
        stick.shown_range = range;
    }

    let alive: Vec<(Entity, Vec2)> = enemies
        .iter()
        .filter(|(_, _, health)| !health.is_dead())
        .map(|(entity, transform, _)| (entity, Vec2::new(transform.translation.x, transform.translation.z) - me))
        .collect();
    let tilt = input.aim.length();
    let previous = stick.locked;

    if let (Some(point), false) = (input.point_world, targeting.open) {
        // Палец указал точку попадания: прицел сразу там, без помощи и сглаживания.
        let offset = (point - me).clamp_length_max(MAX_AIM_RANGE);
        stick.locked = None;
        if offset.length() > 0.1 {
            stick.heading = offset.to_angle();
            stick.shown_heading = stick.heading;
        }
        stick.range = offset.length();
        stick.shown_range = stick.range;
        stick.manual_range = stick.range;
    } else if input.cursor || targeting.open {
        // Курсор: точка едет по земле, без притяжения к врагу и без сглаживания. В наведении
        // авиаудара стик сразу ведёт курсор, вдвое быстрее и до дальности наведения.
        let (speed, range) = if targeting.open {
            (2.0, crate::airstrike::TARGET_RANGE)
        } else {
            (1.0, MAX_AIM_RANGE)
        };
        stick.locked = None;
        let offset = Vec2::from_angle(stick.shown_heading) * stick.shown_range;
        let near_enemy = alive
            .iter()
            .any(|(_, enemy)| enemy.distance(offset) <= CURSOR_FRICTION_RADIUS);
        let moved = (offset + cursor_step(input.aim_world, dt * speed, near_enemy, assist) + input.cursor_shift)
            .clamp_length_max(range);
        let length = moved.length();
        if length > 0.1 {
            stick.heading = moved.to_angle();
            stick.shown_heading = stick.heading;
        }
        stick.range = length;
        stick.shown_range = length;
        stick.manual_range = length;
    } else if input.snap {
        // Короткое касание: сразу на ближайшего видимого врага, без сглаживания курса.
        let eye = rover.translation + Vec3::Y * EYE_HEIGHT;
        let visible = |offset: Vec2| {
            let at = me + offset;
            let target = Vec3::new(at.x, terrain.height_at(at.x, at.y) + TARGET_HEIGHT, at.y);
            steering::has_los(&terrain, obstacles.along(eye, target), eye, target)
        };
        if let Some((entity, offset)) = nearest_visible(&alive, visible) {
            stick.locked = Some(entity);
            stick.heading = offset.to_angle();
            stick.shown_heading = stick.heading;
            stick.range = offset.length();
            stick.shown_range = stick.range;
            if matches!(*weapon, CurrentWeapon::Mortar | CurrentWeapon::Airstrike) {
                stick.manual_range = stick.range;
            }
        }
    } else if let (Some(fraction), CurrentWeapon::Mortar | CurrentWeapon::Airstrike, true) =
        (input.aim_range, *weapon, tilt > 0.0)
    {
        // Сенсорный стик у мортиры: курс по оттяжке, дальность — по её длине.
        stick.locked = None;
        stick.heading = input.aim_world.normalize_or(Vec2::NEG_Y).to_angle();
        stick.range = TOUCH_MIN_RANGE + (MAX_AIM_RANGE - TOUCH_MIN_RANGE) * fraction;
        stick.manual_range = stick.range;
    } else if accept_heading(tilt, stick.last_tilt, dt) {
        let dir = input.aim_world.normalize_or(Vec2::NEG_Y);
        let eye = rover.translation + Vec3::Y * EYE_HEIGHT;
        let visible = |offset: Vec2| {
            let at = me + offset;
            let target = Vec3::new(at.x, terrain.height_at(at.x, at.y) + TARGET_HEIGHT, at.y);
            steering::has_los(&terrain, obstacles.along(eye, target), eye, target)
        };
        match pick_target(dir, &alive, stick.locked, assist, visible) {
            Some((entity, offset)) => {
                stick.locked = Some(entity);
                stick.heading = offset.to_angle();
                stick.range = offset.length();
                if matches!(*weapon, CurrentWeapon::Mortar | CurrentWeapon::Airstrike) {
                    stick.manual_range = stick.range;
                }
            }
            None => {
                stick.locked = None;
                stick.heading = dir.to_angle();
                stick.range = weapon_range(*weapon, stick.manual_range);
            }
        }
    } else {
        // Стик отпущен или отпускается: прицел остаётся, где был, захват снимается.
        stick.locked = None;
    }
    stick.last_tilt = tilt;

    let heading_k = 1.0 - (-HEADING_SMOOTH * dt).exp();
    let range_k = 1.0 - (-RANGE_SMOOTH * dt).exp();
    stick.shown_heading += angle_diff(stick.heading, stick.shown_heading) * heading_k;
    stick.shown_range += (stick.range - stick.shown_range) * range_k;

    if stick.locked.is_some() && stick.locked != previous {
        rumble.kick(0.0, LOCK_RUMBLE);
    }

    let point = me + Vec2::from_angle(stick.shown_heading) * stick.shown_range;
    *aim = AimPoint {
        position: Vec3::new(point.x, terrain.height_at(point.x, point.y), point.y),
        at_limit: false,
        locked: stick.locked.is_some(),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    const NORMAL: AimAssist = AimAssist {
        catch_width: 2.5,
        keep_width: 4.0,
        cursor_friction: 0.4,
    };

    #[test]
    fn враг_в_3_5_м_ловится_на_лёгкой_и_не_ловится_на_нормальной() {
        use crate::difficulty::Difficulty;
        let side = [(entity(1), Vec2::new(20.0, 3.5))];
        let easy = pick_target(Vec2::X, &side, None, Difficulty::Easy.aim_assist(), |_| true);
        assert_eq!(easy.map(|(e, _)| e), Some(entity(1)));
        assert!(pick_target(Vec2::X, &side, None, Difficulty::Normal.aim_assist(), |_| true).is_none());
        assert_eq!(Difficulty::Normal.aim_assist(), NORMAL);
        // Над врагом курсор на лёгкой медленнее, чем на тяжёлой.
        let easy = cursor_step(Vec2::X, 0.1, true, Difficulty::Easy.aim_assist()).length();
        let hard = cursor_step(Vec2::X, 0.1, true, Difficulty::Hard.aim_assist()).length();
        assert!(easy < hard);
    }

    fn entity(n: u32) -> Entity {
        Entity::from_raw_u32(n).unwrap()
    }

    #[test]
    fn касание_берёт_ближайшего_видимого() {
        let near_hidden = (entity(1), Vec2::new(8.0, 0.0));
        let far = (entity(2), Vec2::new(-20.0, 0.0));
        let too_close = (entity(3), Vec2::new(1.0, 0.0));
        let found = nearest_visible(&[far, near_hidden, too_close], |offset| offset.x < 0.0);
        assert_eq!(found.map(|(e, _)| e), Some(entity(2)));
        assert!(nearest_visible(&[], |_| true).is_none());
    }

    #[test]
    fn отпускание_стика_не_меняет_курс() {
        let dt = 1.0 / 60.0;
        // Пружина: 1.0 → 0.6 → 0.25 → отскок 0.3 в другую сторону.
        assert!(!accept_heading(0.6, 1.0, dt));
        assert!(!accept_heading(0.25, 0.6, dt));
        assert!(!accept_heading(0.3, 0.25, dt));
        // Уверенный наклон и поворот по краю.
        assert!(accept_heading(1.0, 1.0, dt));
        assert!(accept_heading(0.5, 0.2, dt));
    }

    #[test]
    fn помощь_ловит_далёкого_врага_в_полосе_и_не_ловит_сзади() {
        let far = (entity(1), Vec2::new(35.0, 1.5));
        let behind = (entity(2), Vec2::new(-10.0, 0.0));
        let found = pick_target(Vec2::X, &[far, behind], None, NORMAL, |_| true);
        assert_eq!(found.map(|(e, _)| e), Some(entity(1)));
        assert!(pick_target(Vec2::X, &[behind], None, NORMAL, |_| true).is_none());
        // За домом не ловится.
        assert!(pick_target(Vec2::X, &[far], None, NORMAL, |_| false).is_none());
    }

    #[test]
    fn пойманный_враг_держится_пока_другой_не_заметно_лучше() {
        let held = (entity(1), Vec2::new(20.0, 2.0));
        let slightly_better = (entity(2), Vec2::new(20.0, 1.5));
        let found = pick_target(Vec2::X, &[held, slightly_better], Some(entity(1)), NORMAL, |_| true);
        assert_eq!(found.map(|(e, _)| e), Some(entity(1)));
        let much_better = (entity(3), Vec2::new(20.0, 0.0));
        let found = pick_target(Vec2::X, &[held, much_better], Some(entity(1)), NORMAL, |_| true);
        assert_eq!(found.map(|(e, _)| e), Some(entity(3)));
    }
}
