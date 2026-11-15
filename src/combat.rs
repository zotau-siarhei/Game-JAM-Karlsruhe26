//! Общие части боя: здоровье, команды, коллайдеры, урон и гибель.
//!
//! Всё, что ранит, отправляет сообщение [`Damage`] и само здоровье не трогает:
//!
//! ```ignore
//! fn bullet_hit(mut damage: MessageWriter<Damage>) {
//!     damage.write(Damage { target, amount: 6.0, source_team: Team::Player });
//! }
//! ```
//!
//! Одна система применяет урон, ограничивает здоровье, даёт вспышку корпуса и один раз
//! отправляет [`Died`], когда здоровье кончилось.

use bevy::prelude::*;

use crate::explosion::Explosion;
use crate::rover::Rover;
use crate::run::GameState;

/// Здоровье лунохода.
pub const ROVER_MAX_HEALTH: f32 = 1500.0;

/// Радиус коллайдера лунохода, м.
pub const ROVER_COLLIDER_RADIUS: f32 = 1.2;

/// Урон в центре взрыва мощностью 1: снаряд мортиры.
pub const EXPLOSION_DAMAGE: f32 = 100.0;

/// Доля урона от взрыва своей команды: свой выстрел ранит слабо и не убивает.
pub const FRIENDLY_FIRE: f32 = 0.15;

/// Сколько секунд корпус светится после попадания.
const FLASH_SECONDS: f32 = 0.12;

/// Подключает урон, гибель и время выживания.
pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SurvivalTime>()
            .add_message::<Damage>()
            .add_message::<Died>()
            .add_systems(Startup, create_flash_material)
            .add_systems(
                Update,
                (
                    equip_rover,
                    explosion_damage,
                    apply_damage,
                    update_flash,
                    count_survival_time.run_if(in_state(GameState::Playing)),
                )
                    .chain(),
            );
    }
}

/// Здоровье сущности.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Health {
    /// Полное здоровье.
    pub fn new(max: f32) -> Self {
        Self { current: max, max }
    }

    /// Отнимает здоровье, не опуская его ниже нуля. Отрицательный урон не лечит.
    pub fn damage(&mut self, amount: f32) {
        self.current = (self.current - amount.max(0.0)).clamp(0.0, self.max);
    }

    /// Добавляет здоровье, не поднимая его выше максимума. Погибшего не воскрешает.
    pub fn heal(&mut self, amount: f32) {
        if self.is_dead() {
            return;
        }
        self.current = (self.current + amount.max(0.0)).clamp(0.0, self.max);
    }

    /// Доля оставшегося здоровья от 0 до 1.
    pub fn fraction(&self) -> f32 {
        if self.max <= 0.0 {
            return 0.0;
        }
        (self.current / self.max).clamp(0.0, 1.0)
    }

    pub fn is_dead(&self) -> bool {
        self.current <= 0.0
    }
}

/// Сторона в бою. Свой урон ослабляется, по чужим стреляют.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Team {
    Player,
    Enemy,
    /// Ничьи: жители. Их ранят все, своего ослабленного урона у них нет.
    Neutral,
}

/// Круглый коллайдер для попаданий и тарана.
#[derive(Component, Debug, Clone, Copy)]
pub struct Collider {
    // Радиус читают пули и таран, которых пока нет.
    #[allow(dead_code)]
    /// Радиус, м.
    pub radius: f32,
}

/// Урон по сущности со здоровьем.
#[derive(Message, Debug, Clone, Copy)]
pub struct Damage {
    /// Кому.
    pub target: Entity,
    /// Сколько здоровья отнять.
    pub amount: f32,
    // Команду источника прочитают тревога и счёт убийств.
    #[allow(dead_code)]
    /// Чья атака.
    pub source_team: Team,
}

/// Здоровье сущности кончилось. Отправляется один раз.
#[derive(Message, Debug, Clone, Copy)]
pub struct Died {
    pub entity: Entity,
}

/// Время выживания в текущем забеге, с. Растёт, пока луноход жив.
#[derive(Resource, Default, Debug)]
pub struct SurvivalTime(pub f32);

/// Материал вспышки при попадании.
#[derive(Resource)]
struct FlashMaterial(Handle<StandardMaterial>);

/// Корпус светится после попадания.
#[derive(Component)]
pub(crate) struct HitFlash {
    remaining: f32,
    /// Материал, который вернётся после вспышки.
    original: Handle<StandardMaterial>,
}

/// Цель из модели: при попадании вспыхивают все меши под ней, а не только её собственный.
///
/// У модели материалы лежат на дочерних сущностях сцены, у самой цели меша нет.
#[derive(Component, Default)]
pub struct FlashWhole;

fn create_flash_material(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        emissive: LinearRgba::rgb(4.0, 4.0, 4.0),
        ..default()
    });
    commands.insert_resource(FlashMaterial(material));
}

/// Даёт луноходу здоровье, команду и коллайдер, как только он появился.
fn equip_rover(mut commands: Commands, rovers: Query<Entity, Added<Rover>>) {
    for rover in &rovers {
        commands.entity(rover).insert((
            Health::new(ROVER_MAX_HEALTH),
            Team::Player,
            Collider {
                radius: ROVER_COLLIDER_RADIUS * crate::rover::ROVER_SCALE,
            },
        ));
    }
}

/// Урон взрыва мощностью `power` и радиусом `radius` на расстоянии `distance` от центра.
///
/// Спадает к краю как `(1 - d/R)²`. Своей команде достаётся [`FRIENDLY_FIRE`] от урона.
pub fn blast_damage(power: f32, radius: f32, distance: f32, friendly: bool) -> f32 {
    if radius <= 0.0 || distance >= radius {
        return 0.0;
    }
    let falloff = 1.0 - (distance / radius).max(0.0);
    let damage = EXPLOSION_DAMAGE * power * falloff * falloff;
    if friendly { damage * FRIENDLY_FIRE } else { damage }
}

/// Взрыв ранит всех со здоровьем в своём радиусе.
fn explosion_damage(
    mut explosions: MessageReader<Explosion>,
    targets: Query<(Entity, &GlobalTransform, &Team, &Health)>,
    mut damage: MessageWriter<Damage>,
) {
    for explosion in explosions.read() {
        for (target, transform, team, health) in &targets {
            if health.is_dead() {
                continue;
            }
            let distance = transform.translation().distance(explosion.position);
            let amount = blast_damage(
                explosion.power,
                explosion.radius,
                distance,
                *team == explosion.source_team,
            );
            if amount > 0.0 {
                damage.write(Damage {
                    target,
                    amount,
                    source_team: explosion.source_team,
                });
            }
        }
    }
}

/// Применяет урон, запускает вспышку корпуса и сообщает о гибели.
fn apply_damage(
    mut commands: Commands,
    mut damages: MessageReader<Damage>,
    mut died: MessageWriter<Died>,
    flash: Option<Res<FlashMaterial>>,
    mut targets: Query<(
        &mut Health,
        Option<&mut MeshMaterial3d<StandardMaterial>>,
        Option<&mut HitFlash>,
        Has<FlashWhole>,
    )>,
    children: Query<&Children>,
    mut parts: Query<(&mut MeshMaterial3d<StandardMaterial>, Option<&mut HitFlash>), Without<Health>>,
) {
    for damage in damages.read() {
        let Ok((mut health, material, flashing, whole)) = targets.get_mut(damage.target) else {
            continue;
        };
        if health.is_dead() || damage.amount <= 0.0 {
            continue;
        }
        health.damage(damage.amount);
        if health.is_dead() {
            died.write(Died {
                entity: damage.target,
            });
        }

        let Some(flash) = flash.as_ref() else {
            continue;
        };
        // Модель: вспыхивает каждый меш под целью и потом получает свой материал с текстурой.
        if whole {
            for part in children.iter_descendants(damage.target) {
                let Ok((mut material, flashing)) = parts.get_mut(part) else {
                    continue;
                };
                if let Some(mut flashing) = flashing {
                    flashing.remaining = FLASH_SECONDS;
                } else {
                    let original = std::mem::replace(&mut material.0, flash.0.clone());
                    commands.entity(part).try_insert(HitFlash {
                        remaining: FLASH_SECONDS,
                        original,
                    });
                }
            }
            continue;
        }
        // Вспышка: корпус на мгновение получает светящийся материал.
        if let Some(mut flashing) = flashing {
            flashing.remaining = FLASH_SECONDS;
        } else if let Some(mut material) = material {
            let original = std::mem::replace(&mut material.0, flash.0.clone());
            commands.entity(damage.target).try_insert(HitFlash {
                remaining: FLASH_SECONDS,
                original,
            });
        }
    }
}

/// Возвращает корпусу его материал, когда вспышка закончилась.
fn update_flash(
    mut commands: Commands,
    time: Res<Time>,
    mut flashing: Query<(Entity, &mut HitFlash, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    for (entity, mut flash, mut material) in &mut flashing {
        flash.remaining -= time.delta_secs();
        if flash.remaining <= 0.0 {
            material.0 = flash.original.clone();
            commands.entity(entity).try_remove::<HitFlash>();
        }
    }
}

/// Считает время выживания, пока луноход жив.
fn count_survival_time(
    time: Res<Time>,
    mut survival: ResMut<SurvivalTime>,
    rover: Query<&Health, With<Rover>>,
) {
    if rover.iter().any(|health| !health.is_dead()) {
        survival.0 += time.delta_secs();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_stays_between_zero_and_max() {
        let mut health = Health::new(100.0);
        health.heal(50.0);
        assert_eq!(health.current, 100.0);
        health.damage(30.0);
        assert_eq!(health.current, 70.0);
        health.damage(1000.0);
        assert_eq!(health.current, 0.0);
        assert!(health.is_dead());
        assert_eq!(health.fraction(), 0.0);
    }

    #[test]
    fn negative_amounts_do_nothing_and_dead_is_not_healed() {
        let mut health = Health::new(100.0);
        health.damage(-20.0);
        assert_eq!(health.current, 100.0);
        health.damage(40.0);
        health.heal(-10.0);
        assert_eq!(health.current, 60.0);
        assert_eq!(health.fraction(), 0.6);
        health.damage(60.0);
        health.heal(50.0);
        assert!(health.is_dead());
    }

    #[test]
    fn blast_is_stronger_in_the_centre_than_at_the_edge() {
        let centre = blast_damage(1.0, 5.0, 0.0, false);
        let middle = blast_damage(1.0, 5.0, 2.5, false);
        let edge = blast_damage(1.0, 5.0, 4.9, false);
        assert!(centre > middle && middle > edge && edge > 0.0);
        assert_eq!(blast_damage(1.0, 5.0, 5.0, false), 0.0);
        assert_eq!(blast_damage(1.0, 5.0, 12.0, false), 0.0);
        // Мощный взрыв бьёт сильнее.
        assert!(blast_damage(2.0, 5.0, 1.0, false) > blast_damage(1.0, 5.0, 1.0, false));
    }

    #[test]
    fn own_mortar_shell_under_the_wheels_takes_less_than_a_fifth_of_health() {
        // Взрыв мортиры: мощность 1, радиус 5, центр под луноходом.
        let own = blast_damage(1.0, 5.0, 0.0, true);
        assert!(own > 0.0);
        assert!(own < ROVER_MAX_HEALTH * 0.2, "{own}");
        // Тот же взрыв убивает таран-стрелка с 30 здоровья на расстоянии до двух метров.
        assert!(blast_damage(1.0, 5.0, 2.0, false) >= 30.0);
    }

    #[test]
    fn explosion_damages_everyone_in_radius_and_spares_its_own_team() {
        let mut app = app_with_damage();
        app.add_message::<Explosion>()
            .add_systems(Update, explosion_damage.before(apply_damage));
        let spawn = |app: &mut App, x: f32, team: Team| {
            app.world_mut()
                .spawn((GlobalTransform::from_xyz(x, 0.0, 0.0), team, Health::new(100.0)))
                .id()
        };
        let near_enemy = spawn(&mut app, 1.0, Team::Enemy);
        let far_enemy = spawn(&mut app, 20.0, Team::Enemy);
        let player = spawn(&mut app, 1.0, Team::Player);
        app.world_mut().write_message(Explosion {
            position: Vec3::ZERO,
            radius: 5.0,
            power: 1.0,
            source_team: Team::Player,
        });
        app.update();
        let health = |app: &App, entity| app.world().get::<Health>(entity).unwrap().current;
        assert!(health(&app, near_enemy) < 50.0);
        assert_eq!(health(&app, far_enemy), 100.0);
        // Своя команда ранена, но гораздо слабее.
        assert!(health(&app, player) < 100.0);
        assert!(health(&app, player) > 85.0);
    }

    /// Приложение с одной системой урона.
    fn app_with_damage() -> App {
        let mut app = App::new();
        app.add_message::<Damage>()
            .add_message::<Died>()
            .init_resource::<Deaths>()
            .add_systems(Update, (apply_damage, collect_deaths).chain());
        app
    }

    #[derive(Resource, Default)]
    struct Deaths(Vec<Entity>);

    fn collect_deaths(mut died: MessageReader<Died>, mut deaths: ResMut<Deaths>) {
        deaths.0.extend(died.read().map(|message| message.entity));
    }

    fn hit(app: &mut App, target: Entity, amount: f32) {
        app.world_mut().write_message(Damage {
            target,
            amount,
            source_team: Team::Enemy,
        });
        app.update();
    }

    #[test]
    fn damage_message_lowers_health() {
        let mut app = app_with_damage();
        let target = app.world_mut().spawn(Health::new(100.0)).id();
        hit(&mut app, target, 25.0);
        assert_eq!(app.world().get::<Health>(target).unwrap().current, 75.0);
        assert!(app.world().resource::<Deaths>().0.is_empty());
    }

    #[test]
    fn death_is_reported_once() {
        let mut app = app_with_damage();
        let target = app.world_mut().spawn(Health::new(30.0)).id();
        hit(&mut app, target, 20.0);
        hit(&mut app, target, 20.0);
        hit(&mut app, target, 20.0);
        assert_eq!(app.world().get::<Health>(target).unwrap().current, 0.0);
        assert_eq!(app.world().resource::<Deaths>().0, vec![target]);
    }

    #[test]
    fn hull_flashes_and_gets_its_material_back() {
        let mut app = app_with_damage();
        app.init_resource::<Time>().add_systems(Update, update_flash.after(apply_damage));
        let mut materials = Assets::<StandardMaterial>::default();
        let hull = materials.add(StandardMaterial::default());
        let flash = materials.add(StandardMaterial::default());
        app.insert_resource(FlashMaterial(flash.clone()));
        let target = app
            .world_mut()
            .spawn((Health::new(100.0), MeshMaterial3d(hull.clone())))
            .id();

        hit(&mut app, target, 10.0);
        let material = |app: &App| app.world().get::<MeshMaterial3d<StandardMaterial>>(target).unwrap().0.clone();
        assert_eq!(material(&app), flash);

        // Второе попадание во время вспышки не теряет исходный материал.
        hit(&mut app, target, 10.0);
        assert_eq!(material(&app), flash);

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs_f32(FLASH_SECONDS * 2.0));
        app.update();
        assert_eq!(material(&app), hull);
        assert!(app.world().get::<HitFlash>(target).is_none());
    }

    #[test]
    fn model_flashes_all_its_meshes_and_gets_textures_back() {
        let mut app = app_with_damage();
        app.init_resource::<Time>().add_systems(Update, update_flash.after(apply_damage));
        let mut materials = Assets::<StandardMaterial>::default();
        let texture = materials.add(StandardMaterial::default());
        let flash = materials.add(StandardMaterial::default());
        app.insert_resource(FlashMaterial(flash.clone()));
        let target = app.world_mut().spawn((Health::new(100.0), FlashWhole)).id();
        let scene = app.world_mut().spawn(ChildOf(target)).id();
        let part = app.world_mut().spawn((MeshMaterial3d(texture.clone()), ChildOf(scene))).id();

        hit(&mut app, target, 10.0);
        let material = |app: &App| app.world().get::<MeshMaterial3d<StandardMaterial>>(part).unwrap().0.clone();
        assert_eq!(material(&app), flash);

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs_f32(FLASH_SECONDS * 2.0));
        app.update();
        assert_eq!(material(&app), texture);
    }

    #[test]
    fn damage_to_entity_without_health_is_ignored() {
        let mut app = app_with_damage();
        let target = app.world_mut().spawn_empty().id();
        hit(&mut app, target, 20.0);
        assert!(app.world().resource::<Deaths>().0.is_empty());
    }
}
