//! Модель повреждений в стиле WoT (Этап 3): общее ХП танка + ХП модулей, ремонт, пожар, подрыв БК.

use std::collections::VecDeque;

use bevy::prelude::*;

use super::modules::{ModuleKind, ModuleState, TankModules};
use super::{TankActionKind, TankActionMessage, TankDestroyedMessage};
use crate::ballistics::penetration::{ImpactOutcome, evaluate_impact, penetration_at};
use crate::ballistics::shell::ShellSpec;
use crate::ballistics::{ArmorImpactMessage, HitKind, HitLog};
use crate::core::{DamageRulesMode, Team};
use crate::tank::{PlayerTank, TankCombat, TurretPivot};
use crate::voxel::{Material, Rng, VoxelAssets, spawn_debris_burst};

/// Общее здоровье танка (используется в режиме `ArcadeHpAndModules`).
#[derive(Component, Debug, Clone, Copy)]
pub struct TankHealth {
    pub hp: f32,
    pub max_hp: f32,
}

impl TankHealth {
    pub fn new(max_hp: f32) -> Self {
        Self {
            hp: max_hp,
            max_hp,
        }
    }

    pub fn ratio(&self) -> f32 {
        if self.max_hp <= 0.0 {
            0.0
        } else {
            (self.hp / self.max_hp).clamp(0.0, 1.0)
        }
    }
}

/// Уровень цвета маркера пробития под прицелом (зелёный / жёлтый / красный).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PenIndicator {
    None,
    Green,
    Yellow,
    Red,
}

/// Классификация вероятности пробития по формуле сплошного AP (`SolidAP`).
pub fn classify_penetration(
    shell: &ShellSpec,
    dist_m: f32,
    armor_mm: f32,
    angle_deg: f32,
) -> PenIndicator {
    if angle_deg > shell.ricochet_deg {
        return PenIndicator::Red;
    }
    let pen = penetration_at(shell, dist_m);
    match evaluate_impact(shell, dist_m, armor_mm, angle_deg) {
        ImpactOutcome::Ricochet => PenIndicator::Red,
        ImpactOutcome::Penetration { effective_mm } => {
            if effective_mm <= pen * 0.88 {
                PenIndicator::Green
            } else {
                PenIndicator::Yellow
            }
        }
        ImpactOutcome::NoPenetration { effective_mm } => {
            if effective_mm <= pen * 1.05 {
                PenIndicator::Yellow
            } else {
                PenIndicator::Red
            }
        }
    }
}

/// Всплывающее сообщение об уроне / критическом попадании (HUD).
#[derive(Debug, Clone)]
pub struct DamageFeedEntry {
    pub text: String,
    pub critical: bool,
    pub age: f32,
}

#[derive(Resource, Debug, Default)]
pub struct DamageFeed {
    pub items: VecDeque<DamageFeedEntry>,
}

impl DamageFeed {
    pub fn push(&mut self, text: impl Into<String>, critical: bool) {
        self.items.push_back(DamageFeedEntry {
            text: text.into(),
            critical,
            age: 0.0,
        });
        while self.items.len() > 4 {
            self.items.pop_front();
        }
    }
}

/// Уничтожение танка: отключает управление и выбрасывает воксельные обломки.
pub fn destroy_tank(
    commands: &mut Commands,
    tank_pos: Vec3,
    combat: &mut TankCombat,
    ammo_detonated: bool,
    assets: Option<&VoxelAssets>,
    rng: &mut Rng,
) {
    if !combat.alive {
        return;
    }
    combat.alive = false;
    combat.immobilized = true;
    combat.gun_jammed = true;
    combat.speed_mult = 0.0;
    combat.traverse_mult = 0.0;

    if let Some(assets) = assets {
        let count = if ammo_detonated { 42 } else { 22 };
        let mat = if combat.team == Team::Enemy {
            Material::EnemyMark
        } else {
            Material::TankStar
        };
        spawn_debris_burst(
            commands,
            assets,
            0,
            tank_pos + Vec3::Y * 1.2,
            Vec3::Y,
            mat,
            count,
            0.45,
            rng,
        );
    }
}

/// Обработка пробитий и урона модулям в режиме `ArcadeHpAndModules` (WoT).
#[allow(clippy::too_many_arguments)]
pub fn handle_arcade_hp_impacts(
    mut commands: Commands,
    mode: Res<DamageRulesMode>,
    mut impacts: MessageReader<ArmorImpactMessage>,
    player_q: Query<Entity, With<PlayerTank>>,
    mut tanks: Query<(
        &GlobalTransform,
        &mut TankHealth,
        &mut TankModules,
        &mut TankCombat,
        Option<&PlayerTank>,
    )>,
    turrets: Query<(&GlobalTransform, &TurretPivot)>,
    mut log: ResMut<HitLog>,
    mut feed: ResMut<DamageFeed>,
    assets: Option<Res<VoxelAssets>>,
    mut destroyed_w: MessageWriter<TankDestroyedMessage>,
    mut action_w: MessageWriter<TankActionMessage>,
    mut seed: Local<u64>,
) {
    if *mode != DamageRulesMode::ArcadeHpAndModules {
        return;
    }

    let human_ent = player_q.single().ok();

    for msg in impacts.read() {
        let Some(tank_ent) = msg.owner_tank else {
            continue;
        };
        let Ok((hull_gt, mut health, mut modules, mut combat, player_opt)) = tanks.get_mut(tank_ent)
        else {
            continue;
        };
        if !combat.alive || msg.shooter_team == Some(combat.team) {
            continue;
        }

        let is_human_shooter = match (human_ent, msg.shooter) {
            (Some(p), Some(s)) => p == s,
            (None, None) => msg.shooter_team == Some(Team::Player),
            _ => false,
        };
        let is_human_victim =
            player_opt.is_some() || (human_ent.is_none() && combat.team == Team::Player);

        match msg.outcome {
            ImpactOutcome::Ricochet => {
                if is_human_shooter {
                    feed.push(format!("{} — РИКОШЕТ", combat.tank_name), false);
                }
                continue;
            }
            ImpactOutcome::NoPenetration { .. } => {
                if is_human_shooter {
                    feed.push(format!("{} — БРОНЯ НЕ ПРОБИТА", combat.tank_name), false);
                }
                continue;
            }
            ImpactOutcome::Penetration { .. } => {}
        }

        *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut rng = Rng::new(*seed | 1);

        let hp_dmg = msg.shell.arcade_damage;
        health.hp = (health.hp - hp_dmg).max(0.0);

        let mut ammo_detonated = false;
        let was_burning = modules.fire_timer > 0.0;
        let mut damaged_names: Vec<&'static str> = Vec::new();
        let turret_gt = turrets
            .iter()
            .find(|(_, tp)| tp.tank == tank_ent)
            .map(|(gt, _)| *gt);
        let local_hit_y = msg.hit_point.y - hull_gt.translation().y;

        // Трассировка остаточного кинетического пробоя сплошного снаряда внутри корпуса/башни.
        let mod_hits = modules.raycast_modules(
            hull_gt,
            turret_gt.as_ref(),
            msg.hit_point,
            msg.ray_dir,
            4.6,
            msg.zone,
            local_hit_y,
        );
        let mod_dmg = (msg.shell.caliber_mm * 1.15).clamp(65.0, 145.0);
        for (i, _) in mod_hits.into_iter().take(2) {
            let m = &mut modules.modules[i];
            let (prev, next) = m.apply_damage(mod_dmg);
            if next != prev {
                damaged_names.push(m.kind.ru_short());
                if m.kind == ModuleKind::AmmoRack && next == ModuleState::Destroyed {
                    ammo_detonated = true;
                }
                if m.kind == ModuleKind::FuelTank
                    && (next == ModuleState::Destroyed || rng.f32() < 0.45)
                {
                    modules.fire_timer = 5.0;
                    if is_human_victim {
                        feed.push("ПОЖАР! Нажмите [6] Огнетушитель", true);
                    }
                }
            }
        }

        if !was_burning && modules.fire_timer > 0.0 && health.hp > 0.0 && !ammo_detonated {
            action_w.write(TankActionMessage {
                tank: tank_ent,
                pos: hull_gt.translation(),
                is_player: player_opt.is_some(),
                kind: TankActionKind::FireStarted,
            });
        }

        if ammo_detonated {
            health.hp = 0.0;
        }

        let zone_ru = msg.zone.map_or("броню", |z| z.ru_name());
        if health.hp <= 0.0 {
            destroy_tank(
                &mut commands,
                hull_gt.translation(),
                &mut combat,
                ammo_detonated,
                assets.as_deref(),
                &mut rng,
            );
            destroyed_w.write(TankDestroyedMessage {
                tank: tank_ent,
                pos: hull_gt.translation(),
                team: combat.team,
                ammo_detonated,
                killer: msg.shooter,
            });
            let reason = if ammo_detonated {
                "ДЕТОНАЦИЯ БОЕУКЛАДКИ"
            } else {
                "УНИЧТОЖЕН"
            };
            log.push(
                HitKind::Penetration,
                format!("{reason}: {} (-{:.0} HP, в {zone_ru})", combat.tank_name, hp_dmg),
            );
            if is_human_shooter || is_human_victim {
                feed.push(format!("{reason}: {}!", combat.tank_name), true);
            }
        } else {
            modules.apply_penalties_to_combat(&mut combat);
            let crit = if damaged_names.is_empty() {
                String::new()
            } else {
                format!(" [{}]", damaged_names.join(", "))
            };
            if is_human_shooter {
                feed.push(
                    format!("-{:.0} HP{} ({})", hp_dmg, crit, zone_ru),
                    !damaged_names.is_empty(),
                );
            } else if is_human_victim && !damaged_names.is_empty() {
                feed.push(format!("Повреждено: {}", damaged_names.join(", ")), true);
            }
        }
    }
}

/// Тикает авторемонт модулей, пожар по ХП и ручные расходники (`[5]` / `[6]`).
#[allow(clippy::too_many_arguments)]
pub fn tick_modules_repair_and_fire(
    mut commands: Commands,
    time: Res<Time>,
    mode: Res<DamageRulesMode>,
    keys: Res<ButtonInput<KeyCode>>,
    player_q: Query<Entity, With<PlayerTank>>,
    mut tanks: Query<(
        Entity,
        &GlobalTransform,
        &mut TankHealth,
        &mut TankModules,
        &mut TankCombat,
        Option<&PlayerTank>,
    )>,
    mut log: ResMut<HitLog>,
    mut feed: ResMut<DamageFeed>,
    assets: Option<Res<VoxelAssets>>,
    mut destroyed_w: MessageWriter<TankDestroyedMessage>,
    mut action_w: MessageWriter<TankActionMessage>,
    mut fire_log_cd: Local<f32>,
    mut seed: Local<u64>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    for entry in feed.items.iter_mut() {
        entry.age += dt;
    }
    while feed.items.front().is_some_and(|e| e.age > 3.5) {
        feed.items.pop_front();
    }

    if *mode == DamageRulesMode::PrototypeTargets {
        return;
    }

    let human_ent = player_q.single().ok();
    *fire_log_cd = (*fire_log_cd - dt).max(0.0);

    for (tank_ent, gt, mut health, mut modules, mut combat, is_player) in &mut tanks {
        if !combat.alive {
            continue;
        }
        let player_flag = is_player.is_some();

        // Расходники игрока: [5] ремкомплект, [6] огнетушитель.
        if player_flag {
            if keys.just_pressed(KeyCode::Digit5) && modules.repair_charges > 0 {
                let needs = modules.modules.iter().any(|m| m.state != ModuleState::Intact);
                if needs {
                    modules.repair_charges -= 1;
                    modules.repair_all();
                    feed.push("Ремкомплект: модули починены", false);
                    action_w.write(TankActionMessage {
                        tank: tank_ent,
                        pos: gt.translation(),
                        is_player: true,
                        kind: TankActionKind::RepairUsed,
                    });
                }
            }
            if keys.just_pressed(KeyCode::Digit6)
                && modules.extinguisher_charges > 0
                && modules.fire_timer > 0.0
            {
                modules.extinguisher_charges -= 1;
                modules.fire_timer = 0.0;
                feed.push("Пожар потушен!", false);
                action_w.write(TankActionMessage {
                    tank: tank_ent,
                    pos: gt.translation(),
                    is_player: true,
                    kind: TankActionKind::FireExtinguished,
                });
            }
        }

        // Авторемонт выбитых модулей.
        for m in &mut modules.modules {
            if m.state == ModuleState::Destroyed && m.repair_timer > 0.0 {
                m.repair_timer -= dt;
                if m.repair_timer <= 0.0 {
                    m.state = ModuleState::Damaged;
                    m.hp = m.max_hp * 0.45;
                    m.repair_timer = 0.0;
                    if player_flag {
                        feed.push(format!("{}: отремонтировано (жёлт.)", m.kind.ru_short()), false);
                        action_w.write(TankActionMessage {
                            tank: tank_ent,
                            pos: gt.translation(),
                            is_player: true,
                            kind: TankActionKind::AutoRepaired,
                        });
                    }
                }
            }
        }

        // Пожар: урон по ХП в аркадном режиме.
        if modules.fire_timer > 0.0 {
            modules.fire_timer = (modules.fire_timer - dt).max(0.0);
            if *mode == DamageRulesMode::ArcadeHpAndModules {
                health.hp = (health.hp - 35.0 * dt).max(0.0);
                if health.hp <= 0.0 {
                    *seed = seed.wrapping_add(1337);
                    let mut rng = Rng::new(*seed);
                    destroy_tank(
                        &mut commands,
                        gt.translation(),
                        &mut combat,
                        false,
                        assets.as_deref(),
                        &mut rng,
                    );
                    destroyed_w.write(TankDestroyedMessage {
                        tank: tank_ent,
                        pos: gt.translation(),
                        team: combat.team,
                        ammo_detonated: false,
                        killer: None,
                    });
                    log.push(
                        HitKind::Penetration,
                        format!("{} сгорел от пожара топливных баков!", combat.tank_name),
                    );
                    if player_flag || human_ent.is_none() {
                        feed.push(format!("{} СГОРЕЛ (пожар)!", combat.tank_name), true);
                    }
                }
            }
        }

        modules.apply_penalties_to_combat(&mut combat);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::core::GameState;
    use crate::tank::ArmorZone;
    use crate::tank::load_tank_spec;
    use bevy::state::app::StatesPlugin;
    use bevy::time::TimeUpdateStrategy;

    #[test]
    fn penetration_indicator_classifies_green_yellow_and_red() {
        let spec = load_tank_spec("t34_85").unwrap();
        // На 100 м пробитие 130 мм.
        assert_eq!(classify_penetration(&spec.shell, 100.0, 45.0, 0.0), PenIndicator::Green);
        assert_eq!(classify_penetration(&spec.shell, 100.0, 125.0, 0.0), PenIndicator::Yellow);
        assert_eq!(classify_penetration(&spec.shell, 100.0, 200.0, 0.0), PenIndicator::Red);
        // Угол 75° -> гарантированный рикошет -> красный.
        assert_eq!(classify_penetration(&spec.shell, 100.0, 10.0, 75.0), PenIndicator::Red);
    }

    fn build_hp_test_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            StatesPlugin,
        ))
        .init_state::<GameState>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<HitLog>()
        .init_resource::<DamageFeed>()
        .insert_resource(DamageRulesMode::ArcadeHpAndModules)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            0.5,
        )))
        .add_message::<ArmorImpactMessage>()
        .add_message::<TankDestroyedMessage>()
        .add_message::<TankActionMessage>()
        .add_systems(
            Update,
            (handle_arcade_hp_impacts, tick_modules_repair_and_fire).chain(),
        );
        app.world_mut()
            .resource_mut::<NextState<GameState>>()
            .set(GameState::InBattle);
        app.finish();
        app.cleanup();
        app
    }

    #[test]
    fn arcade_hp_reduces_hp_damages_modules_and_detonates_ammo_rack() {
        let mut app = build_hp_test_app();
        let spec = load_tank_spec("t34_85").unwrap();
        let enemy = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
                TankHealth::new(spec.max_hp),
                TankModules::for_spec(&spec),
                TankCombat::new(Team::Enemy, spec.name.clone()),
            ))
            .id();
        app.update();

        // Пробитие кормы -> удар по двигателю + снижение общего ХП на 180.
        app.world_mut().write_message(ArmorImpactMessage {
            plate_entity: enemy,
            owner_tank: Some(enemy),
            zone: Some(ArmorZone::HullRear),
            target_name: "Т-34-85 (корма корпуса)".into(),
            shooter: None,
            shooter_team: Some(Team::Player),
            hit_point: Vec3::new(0.0, 0.72, 2.8),
            hit_normal: Vec3::Z,
            ray_dir: Vec3::NEG_Z,
            dist_m: 100.0,
            angle_deg: 0.0,
            armor_thickness_mm: 40.0,
            shell: spec.shell,
            outcome: ImpactOutcome::Penetration { effective_mm: 40.0 },
        });
        app.update();

        {
            let h = app.world().get::<TankHealth>(enemy).unwrap();
            let mods = app.world().get::<TankModules>(enemy).unwrap();
            let c = app.world().get::<TankCombat>(enemy).unwrap();
            assert!((h.hp - (850.0 - 180.0)).abs() < 1e-2, "hp={}", h.hp);
            assert_eq!(mods.state(ModuleKind::Engine), ModuleState::Damaged);
            assert!(c.alive);
            assert!((c.speed_mult - 0.55).abs() < 1e-2);
        }

        // Два попадания по боеукладке -> состояние Destroyed -> детонация (HP -> 0, alive=false).
        for _ in 0..2 {
            app.world_mut().write_message(ArmorImpactMessage {
                plate_entity: enemy,
                owner_tank: Some(enemy),
                zone: Some(ArmorZone::HullSideLeft),
                target_name: "Т-34-85 (левый борт)".into(),
                shooter: None,
                shooter_team: Some(Team::Player),
                hit_point: Vec3::new(-1.3, 0.68, -0.45),
                hit_normal: Vec3::NEG_X,
                ray_dir: Vec3::X,
                dist_m: 100.0,
                angle_deg: 0.0,
                armor_thickness_mm: 45.0,
                shell: spec.shell,
                outcome: ImpactOutcome::Penetration { effective_mm: 45.0 },
            });
            app.update();
        }

        let h = app.world().get::<TankHealth>(enemy).unwrap();
        let mods = app.world().get::<TankModules>(enemy).unwrap();
        let c = app.world().get::<TankCombat>(enemy).unwrap();
        assert_eq!(mods.state(ModuleKind::AmmoRack), ModuleState::Destroyed);
        assert_eq!(h.hp, 0.0);
        assert!(!c.alive);
    }

    #[test]
    fn fire_burns_hp_and_extinguisher_stops_fire() {
        let mut app = build_hp_test_app();
        let spec = load_tank_spec("t34_85").unwrap();
        let mut mods = TankModules::for_spec(&spec);
        mods.fire_timer = 4.0;
        mods.get_mut(ModuleKind::Engine).unwrap().apply_damage(500.0);
        let player = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
                TankHealth::new(spec.max_hp),
                mods,
                TankCombat::new(Team::Player, spec.name.clone()),
                crate::tank::PlayerTank,
            ))
            .id();
        // Первый кадр инициализирует Time, второй даёт dt = 0.5 с.
        app.update();
        app.update();

        let hp_after_burn = app.world().get::<TankHealth>(player).unwrap().hp;
        assert!(hp_after_burn < spec.max_hp, "пожар должен уменьшать ХП: {hp_after_burn}");

        // Игрок нажимает [6] (огнетушитель) и [5] (ремкомплект).
        {
            let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            input.press(KeyCode::Digit6);
            input.press(KeyCode::Digit5);
        }
        app.update();

        let mods_after = app.world().get::<TankModules>(player).unwrap();
        assert_eq!(mods_after.fire_timer, 0.0);
        assert_eq!(mods_after.extinguisher_charges, 1);
        assert_eq!(mods_after.state(ModuleKind::Engine), ModuleState::Intact);
        assert_eq!(mods_after.repair_charges, 1);
    }

    #[test]
    fn damage_feed_ignores_ai_vs_ai_and_attributes_killer() {
        let mut app = build_hp_test_app();
        let spec = load_tank_spec("t34_85").unwrap();
        let human = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
                TankHealth::new(spec.max_hp),
                TankModules::for_spec(&spec),
                TankCombat::new(Team::Player, spec.name.clone()),
                PlayerTank,
            ))
            .id();
        let ally_ai = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
                TankHealth::new(spec.max_hp),
                TankModules::for_spec(&spec),
                TankCombat::new(Team::Player, "Союзник".into()),
            ))
            .id();
        let enemy = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
                TankHealth::new(120.0),
                TankModules::for_spec(&spec),
                TankCombat::new(Team::Enemy, "Враг".into()),
            ))
            .id();
        app.update();

        // Союзный ИИ стреляет по врагу -> лента урона игрока должна оставаться пустой.
        app.world_mut().write_message(ArmorImpactMessage {
            plate_entity: enemy,
            owner_tank: Some(enemy),
            zone: Some(ArmorZone::HullRear),
            target_name: "Враг (корма)".into(),
            shooter: Some(ally_ai),
            shooter_team: Some(Team::Player),
            hit_point: Vec3::new(0.0, 0.72, 2.8),
            hit_normal: Vec3::Z,
            ray_dir: Vec3::NEG_Z,
            dist_m: 100.0,
            angle_deg: 0.0,
            armor_thickness_mm: 40.0,
            shell: ShellSpec {
                arcade_damage: 40.0,
                ..spec.shell
            },
            outcome: ImpactOutcome::Penetration { effective_mm: 40.0 },
        });
        app.update();
        assert!(app.world().resource::<DamageFeed>().items.is_empty());

        // Игрок добивает врага -> запись в DamageFeed + killer == Some(human).
        let mut reader = app
            .world_mut()
            .resource::<Messages<TankDestroyedMessage>>()
            .get_cursor();
        app.world_mut().write_message(ArmorImpactMessage {
            plate_entity: enemy,
            owner_tank: Some(enemy),
            zone: Some(ArmorZone::HullRear),
            target_name: "Враг (корма)".into(),
            shooter: Some(human),
            shooter_team: Some(Team::Player),
            hit_point: Vec3::new(0.0, 0.72, 2.8),
            hit_normal: Vec3::Z,
            ray_dir: Vec3::NEG_Z,
            dist_m: 100.0,
            angle_deg: 0.0,
            armor_thickness_mm: 40.0,
            shell: ShellSpec {
                arcade_damage: 120.0,
                ..spec.shell
            },
            outcome: ImpactOutcome::Penetration { effective_mm: 40.0 },
        });
        app.update();

        assert!(!app.world().resource::<DamageFeed>().items.is_empty());
        let msgs = app.world().resource::<Messages<TankDestroyedMessage>>();
        let killed: Vec<_> = reader.read(msgs).collect();
        assert_eq!(killed.len(), 1);
        assert_eq!(killed[0].tank, enemy);
        assert_eq!(killed[0].killer, Some(human));
    }
}
