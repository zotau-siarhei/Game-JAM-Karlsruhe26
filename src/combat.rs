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
use crate::frame::{GameSet, RunMessages};
use crate::rover::Rover;
use crate::vehicle::LegStance;

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
            .init_resource::<Immortal>()
            .add_run_message::<Damage>()
            .add_run_message::<Died>()
            .add_run_message::<Heal>()
            .add_systems(Startup, create_flash_material)
            .add_systems(
                Update,
                (
                    equip_rover.in_set(GameSet::Reset),
                    count_survival_time.in_set(GameSet::Simulation),
                    (explosion_damage, apply_heal).in_set(GameSet::Damage),
                    apply_damage.in_set(GameSet::ApplyDamage),
                    update_flash.in_set(GameSet::Presentation),
                ),
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
///
/// Кто кого ранит — [`Team::damage_share`]; оружие пропускает сквозь себя тех, кого его
/// команда не ранит, — [`Team::hostile_to`].
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Team {
    Player,
    Enemy,
    /// Служба защиты жителей: атакует только игрока, с врагами друг друга не ранят.
    Service,
    /// Ничьи: жители, лава. Их ранят все, своего ослабленного урона у них нет.
    Neutral,
}

impl Team {
    /// Доля урона, которую атака этой команды наносит цели команды `target`.
    ///
    /// Своя команда получает [`FRIENDLY_FIRE`], но враги ранят друг друга полностью: их пули,
    /// ракеты и взрывы задевают своих (enemy-friendly-fire). Враги и Служба друг друга не ранят,
    /// Служба бережёт жителей. Ничьих ранят все, и ничьё ранит всех полностью.
    pub fn damage_share(self, target: Team) -> f32 {
        use Team::*;
        match (self, target) {
            (Enemy, Service) | (Service, Enemy) | (Service, Neutral) => 0.0,
            (Enemy, Enemy) => 1.0,
            (Neutral, _) | (_, Neutral) => 1.0,
            (attacker, target) if attacker == target => FRIENDLY_FIRE,
            _ => 1.0,
        }
    }

    /// Пуля, ракета или луч этой команды попадает в цель команды `target`, а не пролетает сквозь.
    pub fn hostile_to(self, target: Team) -> bool {
        self.damage_share(target) >= 1.0
    }
}

/// Откуда выпущен снаряд: союзника, внутри которого выстрел начался, — самого стрелка — он не задевает.
#[derive(Component, Debug, Clone, Copy)]
pub struct ShotFrom(pub Vec3);

/// Пропускает ли выстрел команды `shot` цель команды `target` с центром `center` и радиусом
/// `radius`: своего, внутри которого он начался (стрелок или стоящий вплотную), не задевает.
pub fn spared_ally(shot: Team, target: Team, origin: Vec3, center: Vec3, radius: f32) -> bool {
    shot == target && origin.distance(center) <= radius
}

/// Неуязвим для своей команды: так босс не ранит себя ударом приземления, а враги — босса.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct AllyProof;

/// Не вспыхивает от урона: деревьям вспышка не нужна, а горящее дерево от тиков огня моргало бы.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct NoHitFlash;

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
    /// Чья атака.
    pub source_team: Team,
}

/// Здоровье сущности кончилось. Отправляется один раз.
///
/// Сама сущность доживает до набора `GameSet::Cleanup` этого кадра: читатель, которому нужны
/// её компоненты, стоит в `GameSet::Death`. Читателю в `GameSet::Presentation` (звук, эффекты)
/// сущность уже недоступна — для него в сообщении лежат место гибели и команда убившего.
#[derive(Message, Debug, Clone, Copy)]
pub struct Died {
    pub entity: Entity,
    /// Где сущность была в момент гибели.
    pub position: Vec3,
    /// Чья атака добила.
    pub killer: Team,
}

/// Лечение сущности со здоровьем: ремонт и аптечки. Здоровье напрямую не меняют, как и с [`Damage`].
#[derive(Message, Debug, Clone, Copy)]
pub struct Heal {
    /// Кого.
    pub target: Entity,
    /// Сколько здоровья добавить. Выше максимума здоровье не растёт, погибший не оживает.
    pub amount: f32,
}

/// Применяет лечение. Стоит в `GameSet::Damage`: после ремонта и подбора аптечек, до урона.
pub fn apply_heal(mut heals: MessageReader<Heal>, mut targets: Query<&mut Health>) {
    for heal in heals.read() {
        if let Ok(mut health) = targets.get_mut(heal.target) {
            health.heal(heal.amount);
        }
    }
}

/// Время выживания в текущем забеге, с. Растёт, пока луноход жив.
#[derive(Resource, Default, Debug)]
pub struct SurvivalTime(pub f32);

/// Материал вспышки при попадании.
#[derive(Resource)]
struct FlashMaterial(Handle<StandardMaterial>);

/// Корпус светится после попадания: сама сущность и все её потомки с материалом. Так видно
/// и попадание по луноходу, у которого меши лежат в дочерних сущностях модели.
#[derive(Component)]
struct HitFlash {
    remaining: f32,
}

/// Меш светится вспышкой попадания и помнит свой материал.
#[derive(Component)]
pub(crate) struct FlashedMesh {
    /// Материал, который вернётся после вспышки.
    original: Handle<StandardMaterial>,
}

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

/// Броня от лёгких взрывов: взрыв мощностью меньше `light_power` наносит `share` урона.
/// У босса — чтобы ракеты не снимали его за полтора десятка попаданий.
#[derive(Component, Debug, Clone, Copy)]
pub struct BlastArmor {
    pub light_power: f32,
    pub share: f32,
}

impl BlastArmor {
    /// Доля урона взрыва мощностью `power`.
    pub fn share(&self, power: f32) -> f32 {
        if power < self.light_power { self.share } else { 1.0 }
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

/// Доля радиуса взрыва, в которой урон полный: чаша кратера. Вал кратера — уже зона спада.
pub const BLAST_CORE: f32 = 0.75;

/// Во сколько радиусов взрыва кончается урон взрыва с ядром: чуть дальше кратера.
pub const BLAST_REACH: f32 = 1.25;

/// Урон взрыва лунохода по врагу: полный во всей чаше кратера (до [`BLAST_CORE`]·R),
/// дальше спадает как `(1 - t)²` до нуля на [`BLAST_REACH`]·R.
///
/// Так ракета, упавшая рядом с врагом, уничтожает его в своей воронке без точного попадания.
pub fn core_blast_damage(power: f32, radius: f32, distance: f32) -> f32 {
    if radius <= 0.0 {
        return 0.0;
    }
    let core = radius * BLAST_CORE;
    let reach = radius * BLAST_REACH;
    if distance <= core {
        return EXPLOSION_DAMAGE * power;
    }
    if distance >= reach {
        return 0.0;
    }
    let falloff = 1.0 - (distance - core) / (reach - core);
    EXPLOSION_DAMAGE * power * falloff * falloff
}

/// Урон взрыва команды `source` по цели команды `target`: взрывы лунохода бьют врагов и Службу
/// с ядром ([`core_blast_damage`]), остальные — прежним спадом от центра ([`blast_damage`]).
pub fn explosion_damage_to(source: Team, target: Team, power: f32, radius: f32, distance: f32) -> f32 {
    let share = source.damage_share(target);
    let base = if source == Team::Player && matches!(target, Team::Enemy | Team::Service) {
        core_blast_damage(power, radius, distance)
    } else {
        blast_damage(power, radius, distance, false)
    };
    base * share
}

/// Расстояние от взрыва до цели. Машина на ногах может висеть над ямой: взрыв под её корпусом
/// считается по горизонтали, иначе снаряд на дне глубокой ямы её бы не доставал.
fn blast_distance(target: Vec3, blast: Vec3, on_legs: bool) -> f32 {
    if on_legs && blast.y < target.y {
        target.xz().distance(blast.xz())
    } else {
        target.distance(blast)
    }
}

/// Взрыв ранит всех со здоровьем в своём радиусе: свою команду слабо, союзников — никак
/// ([`Team::damage_share`]). Взрывы лунохода бьют врагов полным уроном во всей воронке.
fn explosion_damage(
    mut explosions: MessageReader<Explosion>,
    targets: Query<(Entity, &GlobalTransform, &Team, &Health, Option<&BlastArmor>, Has<LegStance>)>,
    mut damage: MessageWriter<Damage>,
) {
    for explosion in explosions.read() {
        for (target, transform, team, health, armor, on_legs) in &targets {
            if health.is_dead() {
                continue;
            }
            let distance = blast_distance(transform.translation(), explosion.position, on_legs);
            let amount =
                explosion_damage_to(explosion.source_team, *team, explosion.power, explosion.radius, distance)
                    * armor.map_or(1.0, |armor| armor.share(explosion.power));
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
///
/// Единственная система набора `GameSet::ApplyDamage`: до неё — все, кто пишет [`Damage`] в этом
/// кадре, после неё — читатели [`Died`].
/// Режим проверки: луноход не теряет здоровья, можно пройти все волны.
/// Включается параметром запуска: `cargo run -- --immortal`.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Immortal(pub bool);

impl Immortal {
    /// По параметрам запуска: `--immortal` или `--god`.
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Self {
        Self(args.into_iter().any(|a| a == "--immortal" || a == "--god"))
    }
}

fn apply_damage(
    mut commands: Commands,
    immortal: Option<Res<Immortal>>,
    difficulty: Option<Res<crate::difficulty::RunDifficulty>>,
    mut damages: MessageReader<Damage>,
    mut died: MessageWriter<Died>,
    mut targets: Query<(
        &mut Health,
        Option<&GlobalTransform>,
        Has<Rover>,
        Option<&Team>,
        Has<AllyProof>,
        Has<NoHitFlash>,
    )>,
) {
    let immortal = immortal.is_some_and(|i| i.0);
    for damage in damages.read() {
        let Ok((mut health, transform, rover, team, ally_proof, no_flash)) = targets.get_mut(damage.target) else {
            continue;
        };
        if health.is_dead() || damage.amount <= 0.0 {
            continue;
        }
        if ally_proof && team == Some(&damage.source_team) {
            continue;
        }
        // Бессмертный луноход вспыхивает от попаданий, но здоровья не теряет. Урон по луноходу
        // множится по сложности забега, по остальным целям — нет.
        if !(rover && immortal) {
            let scale = match (&difficulty, rover) {
                (Some(difficulty), true) => difficulty.0.damage_to_rover(),
                _ => 1.0,
            };
            health.damage(damage.amount * scale);
        }
        if health.is_dead() {
            died.write(Died {
                entity: damage.target,
                position: transform.map_or(Vec3::ZERO, GlobalTransform::translation),
                killer: damage.source_team,
            });
        }

        // Вспышка: корпус на мгновение получает светящийся материал. Новое попадание продлевает её.
        if !no_flash {
            commands.entity(damage.target).try_insert(HitFlash {
                remaining: FLASH_SECONDS,
            });
        }
    }
}

/// Вспышка попадания: меши сущности и её потомков светятся, потом получают свой материал назад.
///
/// Потомки обходятся каждый кадр вспышки: части модели, загруженные позже, тоже вспыхивают.
fn update_flash(
    mut commands: Commands,
    time: Res<Time>,
    flash: Option<Res<FlashMaterial>>,
    mut flashing: Query<(Entity, &mut HitFlash)>,
    children: Query<&Children>,
    mut meshes: Query<(&mut MeshMaterial3d<StandardMaterial>, Option<&FlashedMesh>)>,
) {
    let Some(flash) = flash else {
        return;
    };
    for (entity, mut hit) in &mut flashing {
        hit.remaining -= time.delta_secs();
        let done = hit.remaining <= 0.0;
        for part in std::iter::once(entity).chain(children.iter_descendants(entity)) {
            let Ok((mut material, flashed)) = meshes.get_mut(part) else {
                continue;
            };
            match flashed {
                Some(flashed) if done => {
                    material.0 = flashed.original.clone();
                    commands.entity(part).try_remove::<FlashedMesh>();
                }
                // Уже светится, в том числе от вспышки предка.
                _ if done || material.0 == flash.0 => {}
                Some(_) => {}
                None => {
                    let original = std::mem::replace(&mut material.0, flash.0.clone());
                    commands.entity(part).try_insert(FlashedMesh { original });
                }
            }
        }
        if done {
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
    #[test]
    fn взрыв_под_машиной_на_ногах_считается_по_горизонтали() {
        let body = Vec3::new(0.0, 10.0, 0.0);
        let under = Vec3::new(3.0, 0.0, 0.0);
        assert_eq!(blast_distance(body, under, true), 3.0);
        assert!(blast_distance(body, under, false) > 10.0);
        // Сверху — как у всех.
        assert_eq!(blast_distance(body, Vec3::new(0.0, 14.0, 0.0), true), 4.0);
    }

    #[test]
    fn бессмертие_включается_параметром_запуска() {
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert!(super::Immortal::from_args(args(&["space_guard", "--immortal"])).0);
        assert!(super::Immortal::from_args(args(&["space_guard", "--god"])).0);
        assert!(!super::Immortal::from_args(args(&["space_guard"])).0);
    }

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

    #[test]
    fn enemies_and_service_spare_each_other_but_both_hurt_the_player() {
        use Team::*;
        assert_eq!(Enemy.damage_share(Service), 0.0);
        assert_eq!(Service.damage_share(Enemy), 0.0);
        assert!(!Enemy.hostile_to(Service) && !Service.hostile_to(Enemy));
        for attacker in [Enemy, Service] {
            assert_eq!(attacker.damage_share(Player), 1.0);
            assert_eq!(Player.damage_share(attacker), 1.0);
            assert!(attacker.hostile_to(Player) && Player.hostile_to(attacker));
        }
        // Враги ранят друг друга полностью, у Службы и лунохода свой урон слабый.
        assert_eq!(Enemy.damage_share(Enemy), 1.0);
        assert!(Enemy.hostile_to(Enemy));
        for team in [Player, Service] {
            assert_eq!(team.damage_share(team), FRIENDLY_FIRE);
            assert!(!team.hostile_to(team));
        }
        // Жителей ранят все, кроме Службы; лава ранит всех.
        assert_eq!(Player.damage_share(Neutral), 1.0);
        assert_eq!(Enemy.damage_share(Neutral), 1.0);
        assert_eq!(Service.damage_share(Neutral), 0.0);
        for target in [Player, Enemy, Service, Neutral] {
            assert_eq!(Neutral.damage_share(target), 1.0);
        }
    }

    #[test]
    fn shot_spares_its_shooter_but_not_other_allies() {
        use Team::*;
        let muzzle = Vec3::new(0.0, 0.9, 0.0);
        assert!(spared_ally(Enemy, Enemy, muzzle, Vec3::ZERO, 3.0));
        assert!(!spared_ally(Enemy, Enemy, muzzle, Vec3::new(8.0, 0.0, 0.0), 3.0));
        // Чужого, стоящего вплотную, выстрел не щадит.
        assert!(!spared_ally(Player, Enemy, muzzle, Vec3::ZERO, 3.0));
    }

    #[test]
    fn ally_proof_ignores_own_team_damage() {
        let mut app = app_with_damage();
        let boss = app.world_mut().spawn((Team::Enemy, AllyProof, Health::new(100.0))).id();
        let current = |app: &App| app.world().get::<Health>(boss).unwrap().current;
        app.world_mut().write_message(Damage { target: boss, amount: 40.0, source_team: Team::Enemy });
        app.update();
        assert_eq!(current(&app), 100.0);
        app.world_mut().write_message(Damage { target: boss, amount: 40.0, source_team: Team::Player });
        app.update();
        assert_eq!(current(&app), 60.0);
    }

    #[test]
    fn enemy_explosion_spares_the_service() {
        let mut app = app_with_damage();
        app.add_message::<Explosion>()
            .add_systems(Update, explosion_damage.before(apply_damage));
        let service = app
            .world_mut()
            .spawn((GlobalTransform::from_xyz(1.0, 0.0, 0.0), Team::Service, Health::new(100.0)))
            .id();
        let player = app
            .world_mut()
            .spawn((GlobalTransform::from_xyz(1.0, 0.0, 0.0), Team::Player, Health::new(100.0)))
            .id();
        app.world_mut().write_message(Explosion {
            position: Vec3::ZERO,
            radius: 5.0,
            power: 1.0,
            source_team: Team::Enemy,
        });
        app.update();
        let health = |app: &App, entity| app.world().get::<Health>(entity).unwrap().current;
        assert_eq!(health(&app, service), 100.0);
        assert!(health(&app, player) < 50.0);
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
        let wheel = materials.add(StandardMaterial::default());
        let flash = materials.add(StandardMaterial::default());
        app.insert_resource(FlashMaterial(flash.clone()));
        let target = app
            .world_mut()
            .spawn((Health::new(100.0), MeshMaterial3d(hull.clone())))
            .id();
        // Деталь модели в дочерней сущности, как у лунохода.
        let part = app
            .world_mut()
            .spawn((MeshMaterial3d(wheel.clone()), ChildOf(target)))
            .id();

        hit(&mut app, target, 10.0);
        let material = |app: &App, entity| {
            app.world().get::<MeshMaterial3d<StandardMaterial>>(entity).unwrap().0.clone()
        };
        assert_eq!(material(&app, target), flash);
        assert_eq!(material(&app, part), flash);

        // Второе попадание во время вспышки не теряет исходный материал.
        hit(&mut app, target, 10.0);
        assert_eq!(material(&app, target), flash);

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs_f32(FLASH_SECONDS * 2.0));
        app.update();
        assert_eq!(material(&app, target), hull);
        assert_eq!(material(&app, part), wheel);
        assert!(app.world().get::<HitFlash>(target).is_none());
    }

    #[test]
    fn tree_does_not_flash() {
        let mut app = app_with_damage();
        let target = app.world_mut().spawn((Health::new(40.0), NoHitFlash)).id();
        hit(&mut app, target, 10.0);
        assert_eq!(app.world().get::<Health>(target).unwrap().current, 30.0, "урон проходит");
        assert!(app.world().get::<HitFlash>(target).is_none(), "без вспышки");
    }

    #[test]
    fn damage_to_entity_without_health_is_ignored() {
        let mut app = app_with_damage();
        let target = app.world_mut().spawn_empty().id();
        hit(&mut app, target, 20.0);
        assert!(app.world().resource::<Deaths>().0.is_empty());
    }

    #[test]
    fn player_blast_deals_full_damage_in_the_whole_crater_bowl() {
        let full = core_blast_damage(1.0, 4.0, 0.0);
        assert_eq!(core_blast_damage(1.0, 4.0, 2.9), full);
        let edge = core_blast_damage(1.0, 4.0, 4.5);
        assert!(edge > 0.0 && edge < full, "{edge}");
        assert_eq!(core_blast_damage(1.0, 4.0, 5.0), 0.0);
    }

    #[test]
    fn enemy_blast_on_rover_keeps_the_old_falloff() {
        for d in [0.0, 1.0, 2.5, 4.0, 6.9] {
            assert_eq!(
                explosion_damage_to(Team::Enemy, Team::Player, 1.4, 7.0, d),
                blast_damage(1.4, 7.0, d, false),
            );
        }
        // Свой взрыв ранит луноход прежней долей и тоже без ядра.
        assert_eq!(
            explosion_damage_to(Team::Player, Team::Player, 1.4, 7.0, 3.0),
            blast_damage(1.4, 7.0, 3.0, false) * FRIENDLY_FIRE,
        );
    }

    #[test]
    fn player_blast_hits_enemies_and_service_with_the_core() {
        let near_miss = explosion_damage_to(Team::Player, Team::Enemy, 0.7, 4.0, 2.0);
        assert_eq!(near_miss, core_blast_damage(0.7, 4.0, 2.0));
        assert_eq!(explosion_damage_to(Team::Player, Team::Service, 0.7, 4.0, 2.0), near_miss);
    }
}
