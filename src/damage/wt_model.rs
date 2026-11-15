//! Модель повреждений в стиле War Thunder (`ModulesOnlyWt`, Этап 6):
//! общее ХП отключено; поражение определяется трассировкой бронебойного сердечника
//! сплошного снаряда `SolidAP`, 3D-конусом осколков (Spall Cone), выведением экипажа
//! (< 2 живых) и подрывом боеукладки.

use bevy::prelude::*;

use super::crew::{CrewRole, CrewState, TankCrew};
use super::hp_model::{DamageFeed, destroy_tank};
use super::modules::{ModuleKind, ModuleState, TankModules};
use super::spall::{compute_residual_penetration, generate_spall_rays, spall_cone_params};
use super::{TankActionKind, TankActionMessage, TankDestroyedMessage};
use crate::ballistics::{ArmorImpactMessage, HitKind, HitLog, penetration::ImpactOutcome};
use crate::core::{DamageRulesMode, Team};
use crate::tank::{PlayerTank, TankCombat, TankRoot, TurretPivot};
use crate::voxel::{Rng, VoxelAssets};

/// Данные последнего попадания для панели WT X-Ray Hit-Cam.
#[derive(Resource, Debug, Clone, Default)]
pub struct XRayHitReport {
    pub active: bool,
    pub age: f32,
    pub victim_name: String,
    pub victim_killed: bool,
    pub outcome_label: String,
    pub armor_zone_ru: String,
    pub nominal_mm: f32,
    pub effective_mm: f32,
    pub residual_pen_mm: f32,
    pub spall_ray_count: usize,
    pub spall_half_angle_deg: f32,
    /// Точка входа в координатах корпуса (`[-1..1]`, X — ширина, Z — длина).
    pub entry_norm_xz: Vec2,
    /// Направление пролёта в плане корпуса (`XZ`).
    pub ray_norm_xz: Vec2,
    pub module_hits: Vec<(ModuleKind, ModuleState)>,
    pub crew_hits: Vec<(CrewRole, CrewState)>,
    pub alive_crew_after: usize,
    pub total_crew: usize,
    pub kill_reason: Option<String>,
}

fn upsert_module_hit(list: &mut Vec<(ModuleKind, ModuleState)>, kind: ModuleKind, st: ModuleState) {
    if let Some((_, existing)) = list.iter_mut().find(|(k, _)| *k == kind) {
        *existing = st;
    } else {
        list.push((kind, st));
    }
}

fn upsert_crew_hit(list: &mut Vec<(CrewRole, CrewState)>, role: CrewRole, st: CrewState) {
    if let Some((_, existing)) = list.iter_mut().find(|(r, _)| *r == role) {
        *existing = st;
    } else {
        list.push((role, st));
    }
}

#[derive(Clone, Copy)]
struct InternalHit {
    t: f32,
    is_crew: bool,
    idx: usize,
}

/// Обработка попаданий в режиме `ModulesOnlyWt` (War Thunder): без уменьшения `TankHealth.hp`.
#[allow(clippy::too_many_arguments)]
pub fn handle_wt_modules_impacts(
    mut commands: Commands,
    mode: Res<DamageRulesMode>,
    mut impacts: MessageReader<ArmorImpactMessage>,
    player_q: Query<Entity, With<PlayerTank>>,
    mut tanks: Query<(
        &GlobalTransform,
        Option<&TankRoot>,
        &mut TankModules,
        &mut TankCrew,
        &mut TankCombat,
        Option<&PlayerTank>,
    )>,
    turrets: Query<(&GlobalTransform, &TurretPivot)>,
    mut log: ResMut<HitLog>,
    mut feed: ResMut<DamageFeed>,
    mut report: ResMut<XRayHitReport>,
    assets: Option<Res<VoxelAssets>>,
    mut destroyed_w: MessageWriter<TankDestroyedMessage>,
    mut action_w: MessageWriter<TankActionMessage>,
    mut seed: Local<u64>,
) {
    if *mode != DamageRulesMode::ModulesOnlyWt {
        return;
    }

    let human_ent = player_q.single().ok();

    for msg in impacts.read() {
        let Some(tank_ent) = msg.owner_tank else {
            continue;
        };
        let Ok((hull_gt, root_opt, mut modules, mut crew, mut combat, player_opt)) =
            tanks.get_mut(tank_ent)
        else {
            continue;
        };
        if !combat.alive || msg.shooter_team == Some(combat.team) {
            continue;
        }
        let was_burning = modules.fire_timer > 0.0;

        let is_human_shooter = match (human_ent, msg.shooter) {
            (Some(p), Some(s)) => p == s,
            (None, None) => msg.shooter_team == Some(Team::Player),
            _ => false,
        };
        let is_human_victim =
            player_opt.is_some() || (human_ent.is_none() && combat.team == Team::Player);
        let record_xray = is_human_shooter || (human_ent.is_none() && msg.shooter.is_none());
        let zone_ru = msg.zone.map_or("Броня", |z| z.ru_name()).to_string();
        let inv_hull = hull_gt.affine().inverse();
        let local_entry = inv_hull.transform_point3(msg.hit_point);
        let local_dir = inv_hull.transform_vector3(msg.ray_dir).normalize_or_zero();
        let (hw, hl) = root_opt.map_or((1.6, 3.0), |r| {
            (
                (r.spec.profile.hull_width * 0.5).max(0.8),
                (r.spec.profile.hull_length * 0.5).max(1.5),
            )
        });
        let entry_norm_xz = Vec2::new(
            (local_entry.x / hw).clamp(-1.0, 1.0),
            (local_entry.z / hl).clamp(-1.0, 1.0),
        );
        let ray_norm_xz = Vec2::new(local_dir.x, local_dir.z).normalize_or_zero();

        match msg.outcome {
            ImpactOutcome::Ricochet => {
                if is_human_shooter {
                    feed.push(format!("{} — РИКОШЕТ", combat.tank_name), false);
                }
                if record_xray {
                    *report = XRayHitReport {
                        active: true,
                        age: 0.0,
                        victim_name: combat.tank_name.clone(),
                        victim_killed: false,
                        outcome_label: "РИКОШЕТ".into(),
                        armor_zone_ru: zone_ru,
                        nominal_mm: msg.armor_thickness_mm,
                        effective_mm: msg.armor_thickness_mm,
                        residual_pen_mm: 0.0,
                        spall_ray_count: 0,
                        spall_half_angle_deg: 0.0,
                        entry_norm_xz,
                        ray_norm_xz,
                        module_hits: Vec::new(),
                        crew_hits: Vec::new(),
                        alive_crew_after: crew.alive_count(),
                        total_crew: crew.total_count(),
                        kill_reason: None,
                    };
                }
                continue;
            }
            ImpactOutcome::NoPenetration { effective_mm } => {
                if is_human_shooter {
                    feed.push(
                        format!("{} — НЕ ПРОБИЛ ({effective_mm:.0}мм)", combat.tank_name),
                        false,
                    );
                }
                if record_xray {
                    *report = XRayHitReport {
                        active: true,
                        age: 0.0,
                        victim_name: combat.tank_name.clone(),
                        victim_killed: false,
                        outcome_label: "НЕ ПРОБИЛ".into(),
                        armor_zone_ru: zone_ru,
                        nominal_mm: msg.armor_thickness_mm,
                        effective_mm,
                        residual_pen_mm: 0.0,
                        spall_ray_count: 0,
                        spall_half_angle_deg: 0.0,
                        entry_norm_xz,
                        ray_norm_xz,
                        module_hits: Vec::new(),
                        crew_hits: Vec::new(),
                        alive_crew_after: crew.alive_count(),
                        total_crew: crew.total_count(),
                        kill_reason: None,
                    };
                }
                continue;
            }
            ImpactOutcome::Penetration { effective_mm } => {
                *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let mut rng = Rng::new(*seed | 1);

                let residual = compute_residual_penetration(
                    &msg.shell,
                    msg.dist_m,
                    msg.armor_thickness_mm,
                    msg.angle_deg,
                );
                let (spall_count, half_angle_deg) =
                    spall_cone_params(&msg.shell, residual, msg.armor_thickness_mm);

                let turret_gt = turrets
                    .iter()
                    .find(|(_, tp)| tp.tank == tank_ent)
                    .map(|(gt, _)| *gt);
                let local_hit_y = msg.hit_point.y - hull_gt.translation().y;

                let mut hit_modules: Vec<(ModuleKind, ModuleState)> = Vec::new();
                let mut hit_crew: Vec<(CrewRole, CrewState)> = Vec::new();
                let mut ammo_detonated = false;

                // 1. Основной сплошной сердечник AP: летит по прямой насквозь, теряя энергию.
                let mut core_hits: Vec<InternalHit> = Vec::new();
                for (idx, t) in modules.raycast_modules(
                    hull_gt,
                    turret_gt.as_ref(),
                    msg.hit_point,
                    msg.ray_dir,
                    4.9,
                    msg.zone,
                    local_hit_y,
                ) {
                    core_hits.push(InternalHit {
                        t,
                        is_crew: false,
                        idx,
                    });
                }
                for (idx, t) in
                    crew.raycast_crew(hull_gt, turret_gt.as_ref(), msg.hit_point, msg.ray_dir, 4.9)
                {
                    core_hits.push(InternalHit {
                        t,
                        is_crew: true,
                        idx,
                    });
                }
                core_hits.sort_by(|a, b| a.t.total_cmp(&b.t));

                let mut core_energy =
                    (msg.shell.caliber_mm * 1.45 + residual * 0.55).clamp(110.0, 270.0);
                for h in core_hits {
                    if core_energy <= 12.0 {
                        break;
                    }
                    if h.is_crew {
                        let member = &mut crew.members[h.idx];
                        let (prev, next) = member.apply_damage(60.0);
                        if next != prev || next == CrewState::Unconscious {
                            upsert_crew_hit(&mut hit_crew, member.role, next);
                        }
                        core_energy -= 38.0;
                    } else {
                        let m = &mut modules.modules[h.idx];
                        let (prev, next) = m.apply_damage(155.0);
                        upsert_module_hit(&mut hit_modules, m.kind, next);
                        if m.kind == ModuleKind::AmmoRack && next == ModuleState::Destroyed {
                            ammo_detonated = true;
                        }
                        if m.kind == ModuleKind::FuelTank
                            && prev != next
                            && (next == ModuleState::Destroyed || rng.f32() < 0.50)
                        {
                            modules.fire_timer = 7.0;
                            if is_human_victim {
                                feed.push("ПОЖАР БАКОВ! Нажмите [6] Огнетушитель", true);
                            }
                        }
                        core_energy -= 48.0;
                    }
                }

                // 2. Вторичный 3D-конус осколков (кинетика SolidAP без ВВ).
                let spall_rays = generate_spall_rays(msg.ray_dir, *seed, spall_count, half_angle_deg);
                let spall_origin = msg.hit_point + msg.ray_dir * 0.12;
                let spall_reach = (1.6 + residual / 60.0).clamp(1.6, 3.4);
                let mod_spall_dmg = (32.0 + msg.shell.caliber_mm * 0.28).clamp(48.0, 82.0);
                let crew_spall_dmg = (24.0 + msg.shell.caliber_mm * 0.16).clamp(34.0, 52.0);

                for ray in &spall_rays {
                    let mh = modules.raycast_modules(
                        hull_gt,
                        turret_gt.as_ref(),
                        spall_origin,
                        *ray,
                        spall_reach,
                        None,
                        local_hit_y,
                    );
                    let ch = crew.raycast_crew(
                        hull_gt,
                        turret_gt.as_ref(),
                        spall_origin,
                        *ray,
                        spall_reach,
                    );
                    let first_mod = mh.first().copied();
                    let first_crew = ch.first().copied();
                    let hit_opt = match (first_mod, first_crew) {
                        (Some((mi, mt)), Some((ci, ct))) => {
                            if ct <= mt {
                                Some(InternalHit {
                                    t: ct,
                                    is_crew: true,
                                    idx: ci,
                                })
                            } else {
                                Some(InternalHit {
                                    t: mt,
                                    is_crew: false,
                                    idx: mi,
                                })
                            }
                        }
                        (Some((mi, mt)), None) => Some(InternalHit {
                            t: mt,
                            is_crew: false,
                            idx: mi,
                        }),
                        (None, Some((ci, ct))) => Some(InternalHit {
                            t: ct,
                            is_crew: true,
                            idx: ci,
                        }),
                        (None, None) => None,
                    };
                    if let Some(h) = hit_opt {
                        if h.is_crew {
                            let member = &mut crew.members[h.idx];
                            let (prev, next) = member.apply_damage(crew_spall_dmg);
                            if next != prev {
                                upsert_crew_hit(&mut hit_crew, member.role, next);
                            }
                        } else {
                            let m = &mut modules.modules[h.idx];
                            let (prev, next) = m.apply_damage(mod_spall_dmg);
                            if next != prev {
                                upsert_module_hit(&mut hit_modules, m.kind, next);
                            }
                            if m.kind == ModuleKind::AmmoRack && next == ModuleState::Destroyed {
                                ammo_detonated = true;
                            }
                            if m.kind == ModuleKind::FuelTank
                                && prev != next
                                && (next == ModuleState::Destroyed || rng.f32() < 0.35)
                            {
                                modules.fire_timer = modules.fire_timer.max(6.5);
                            }
                        }
                    }
                }

                let alive_crew = crew.alive_count();
                let kill_reason = if ammo_detonated {
                    Some("ДЕТОНАЦИЯ БОЕУКЛАДКИ".to_string())
                } else if alive_crew < 2 {
                    Some("ЭКИПАЖ ВЫВЕДЕН ИЗ СТРОЯ".to_string())
                } else {
                    None
                };

                if !was_burning && modules.fire_timer > 0.0 && kill_reason.is_none() {
                    action_w.write(TankActionMessage {
                        tank: tank_ent,
                        pos: hull_gt.translation(),
                        is_player: player_opt.is_some(),
                        kind: TankActionKind::FireStarted,
                    });
                }

                if let Some(ref reason) = kill_reason {
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
                    log.push(
                        HitKind::Penetration,
                        format!("{reason}: {} (WT, {})", combat.tank_name, zone_ru),
                    );
                    if is_human_shooter || is_human_victim {
                        feed.push(format!("{reason}: {}!", combat.tank_name), true);
                    }
                } else {
                    modules.apply_penalties_to_combat(&mut combat);
                    crew.apply_penalties_to_combat(&mut combat);
                    let mut tags: Vec<String> = hit_modules
                        .iter()
                        .map(|(k, _)| k.ru_short().to_string())
                        .collect();
                    for (r, s) in &hit_crew {
                        if *s == CrewState::Unconscious {
                            tags.push(r.ru_short().to_string());
                        }
                    }
                    let summary = if tags.is_empty() {
                        "Пробитие без критических узлов".to_string()
                    } else {
                        tags.join(", ")
                    };
                    if is_human_shooter {
                        feed.push(
                            format!("{} [{}]: {}", combat.tank_name, zone_ru, summary),
                            !tags.is_empty(),
                        );
                    } else if is_human_victim && !tags.is_empty() {
                        feed.push(format!("Выбито: {summary}"), true);
                    }
                }

                if record_xray {
                    *report = XRayHitReport {
                        active: true,
                        age: 0.0,
                        victim_name: combat.tank_name.clone(),
                        victim_killed: kill_reason.is_some(),
                        outcome_label: "ПРОБИТИЕ".into(),
                        armor_zone_ru: zone_ru,
                        nominal_mm: msg.armor_thickness_mm,
                        effective_mm,
                        residual_pen_mm: residual,
                        spall_ray_count: spall_count,
                        spall_half_angle_deg: half_angle_deg,
                        entry_norm_xz,
                        ray_norm_xz,
                        module_hits: hit_modules,
                        crew_hits: hit_crew,
                        alive_crew_after: alive_crew,
                        total_crew: crew.total_count(),
                        kill_reason,
                    };
                }
            }
        }
    }
}

/// Тикает пересадку экипажа (7 с), выгорание от пожара и старение отчёта X-Ray Hit-Cam.
#[allow(clippy::too_many_arguments)]
pub fn tick_wt_crew_and_fire(
    mut commands: Commands,
    time: Res<Time>,
    mode: Res<DamageRulesMode>,
    mut report: ResMut<XRayHitReport>,
    player_q: Query<Entity, With<PlayerTank>>,
    mut tanks: Query<(
        Entity,
        &GlobalTransform,
        &TankModules,
        &mut TankCrew,
        &mut TankCombat,
        Option<&PlayerTank>,
    )>,
    mut log: ResMut<HitLog>,
    mut feed: ResMut<DamageFeed>,
    assets: Option<Res<VoxelAssets>>,
    mut destroyed_w: MessageWriter<TankDestroyedMessage>,
    mut seed: Local<u64>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    if report.active {
        report.age += dt;
        if report.age > 7.5 {
            report.active = false;
        }
    }

    if *mode != DamageRulesMode::ModulesOnlyWt {
        return;
    }

    let human_ent = player_q.single().ok();

    for (tank_ent, gt, modules, mut crew, mut combat, is_player) in &mut tanks {
        if !combat.alive {
            continue;
        }

        // Пожар в режиме WT не отнимает фиктивное общее ХП, но постепенно выводит экипаж.
        let mut burned_out = false;
        if modules.fire_timer > 0.0 {
            crew.fire_damage_acc += dt;
            if crew.fire_damage_acc >= 2.0 {
                crew.fire_damage_acc -= 2.0;
                for role in [
                    CrewRole::Loader,
                    CrewRole::Commander,
                    CrewRole::Driver,
                    CrewRole::Gunner,
                ] {
                    if let Some(m) = crew.member_mut(role)
                        && m.state == CrewState::Alive
                    {
                        m.state = CrewState::Unconscious;
                        m.hp = 0.0;
                        if is_player.is_some() {
                            feed.push(
                                format!("ПОЖАР: {} без сознания! [6] Огнетуш.", role.ru_short()),
                                true,
                            );
                        }
                        break;
                    }
                }
                if crew.alive_count() < 2 {
                    burned_out = true;
                }
            }
        } else {
            crew.fire_damage_acc = 0.0;
        }

        if let Some(replaced_role) = crew.tick_replacement(dt)
            && is_player.is_some()
        {
            feed.push(
                format!("Экипаж пересел на место: {}", replaced_role.ru_short()),
                false,
            );
        }

        if crew.alive_count() < 2 {
            *seed = seed.wrapping_add(4099);
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
            let msg = if burned_out || modules.fire_timer > 0.0 {
                format!("{} сгорел от пожара (WT)!", combat.tank_name)
            } else {
                format!("{} — экипаж выведен из строя!", combat.tank_name)
            };
            log.push(HitKind::Penetration, msg.clone());
            if is_player.is_some() || human_ent.is_none() {
                feed.push(msg, true);
            }
            continue;
        }

        modules.apply_penalties_to_combat(&mut combat);
        crew.apply_penalties_to_combat(&mut combat);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::core::GameState;
    use crate::damage::hp_model::{TankHealth, tick_modules_repair_and_fire};
    use crate::tank::{ArmorZone, load_tank_spec};
    use bevy::state::app::StatesPlugin;
    use bevy::time::TimeUpdateStrategy;

    fn build_wt_test_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin, StatesPlugin))
            .init_state::<GameState>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<HitLog>()
            .init_resource::<DamageFeed>()
            .init_resource::<XRayHitReport>()
            .insert_resource(DamageRulesMode::ModulesOnlyWt)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                0.5,
            )))
            .add_message::<ArmorImpactMessage>()
            .add_message::<TankDestroyedMessage>()
            .add_message::<TankActionMessage>()
            .add_systems(
                Update,
                (
                    handle_wt_modules_impacts,
                    tick_modules_repair_and_fire,
                    tick_wt_crew_and_fire,
                )
                    .chain(),
            );
        app.world_mut()
            .resource_mut::<NextState<GameState>>()
            .set(GameState::InBattle);
        app.finish();
        app.cleanup();
        app
    }

    #[test]
    fn wt_mode_ignores_tank_hp_and_destroys_by_crew_or_ammo_rack() {
        let mut app = build_wt_test_app();
        let spec = load_tank_spec("t34_85").unwrap();
        let enemy = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
                TankRoot {
                    entity: Entity::PLACEHOLDER,
                    spec: spec.clone(),
                    spawn_pos: Vec3::ZERO,
                    spawn_yaw: 0.0,
                },
                TankHealth::new(spec.max_hp),
                TankModules::for_spec(&spec),
                TankCrew::for_spec(&spec),
                TankCombat::new(Team::Enemy, spec.name.clone()),
            ))
            .id();
        app.update();

        // 1. Выстрел по правому борту в корму (двигатель): общее ХП вообще НЕ уменьшается!
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
            let report = app.world().resource::<XRayHitReport>();
            assert_eq!(h.hp, spec.max_hp, "WT-режим не должен трогать общее ХП");
            assert_ne!(mods.state(ModuleKind::Engine), ModuleState::Intact);
            assert!(report.active);
            assert!(report.residual_pen_mm > 50.0);
        }

        // 2. Прямое попадание в боеукладку -> детонация в режиме WT (alive=false), ХП по-прежнему нетронуто.
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

        let h = app.world().get::<TankHealth>(enemy).unwrap();
        let mods = app.world().get::<TankModules>(enemy).unwrap();
        let c = app.world().get::<TankCombat>(enemy).unwrap();
        let report = app.world().resource::<XRayHitReport>();
        assert_eq!(h.hp, spec.max_hp);
        assert_eq!(mods.state(ModuleKind::AmmoRack), ModuleState::Destroyed);
        assert!(!c.alive);
        assert!(report.victim_killed);
        assert_eq!(report.kill_reason.as_deref(), Some("ДЕТОНАЦИЯ БОЕУКЛАДКИ"));
    }

    #[test]
    fn wt_mode_destroys_tank_when_fewer_than_two_crew_survive() {
        let mut app = build_wt_test_app();
        let spec = load_tank_spec("t34_85").unwrap();
        let mut crew = TankCrew::for_spec(&spec);
        crew.member_mut(CrewRole::Commander).unwrap().apply_damage(100.0);
        crew.member_mut(CrewRole::Loader).unwrap().apply_damage(100.0);
        // Осталось ровно 2 (мехвод + наводчик); при выбивании мехвода останется 1 -> уничтожен!
        let enemy = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
                TankHealth::new(spec.max_hp),
                TankModules::for_spec(&spec),
                crew,
                TankCombat::new(Team::Enemy, spec.name.clone()),
            ))
            .id();
        app.update();

        // Попадание в переднюю левую часть корпуса (мехвод).
        app.world_mut().write_message(ArmorImpactMessage {
            plate_entity: enemy,
            owner_tank: Some(enemy),
            zone: Some(ArmorZone::HullFrontUpper),
            target_name: "Т-34-85 (ВЛД)".into(),
            shooter: None,
            shooter_team: Some(Team::Player),
            hit_point: Vec3::new(-0.34, 0.76, -2.5),
            hit_normal: Vec3::NEG_Z,
            ray_dir: Vec3::Z,
            dist_m: 100.0,
            angle_deg: 20.0,
            armor_thickness_mm: 45.0,
            shell: spec.shell,
            outcome: ImpactOutcome::Penetration { effective_mm: 46.0 },
        });
        app.update();

        let c = app.world().get::<TankCombat>(enemy).unwrap();
        let crew_after = app.world().get::<TankCrew>(enemy).unwrap();
        let h = app.world().get::<TankHealth>(enemy).unwrap();
        assert_eq!(crew_after.alive_count(), 1);
        assert!(!c.alive);
        assert_eq!(h.hp, spec.max_hp);
    }

    #[test]
    fn wt_xray_and_feed_ignore_ally_ai_shots() {
        let mut app = build_wt_test_app();
        let spec = load_tank_spec("t34_85").unwrap();
        let human = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
                TankHealth::new(spec.max_hp),
                TankModules::for_spec(&spec),
                TankCrew::for_spec(&spec),
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
                TankCrew::for_spec(&spec),
                TankCombat::new(Team::Player, "Союзник".into()),
            ))
            .id();
        let enemy = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
                TankHealth::new(spec.max_hp),
                TankModules::for_spec(&spec),
                TankCrew::for_spec(&spec),
                TankCombat::new(Team::Enemy, "Враг".into()),
            ))
            .id();
        app.update();

        // Союзный ИИ пробивает кормовой МТО (не смертельно) -> ни XRayHitReport, ни DamageFeed не обновляются.
        app.world_mut().write_message(ArmorImpactMessage {
            plate_entity: enemy,
            owner_tank: Some(enemy),
            zone: Some(ArmorZone::HullSideRight),
            target_name: "Враг (борт МТО)".into(),
            shooter: Some(ally_ai),
            shooter_team: Some(Team::Player),
            hit_point: Vec3::new(1.1, 0.72, 1.9),
            hit_normal: Vec3::X,
            ray_dir: Vec3::NEG_X,
            dist_m: 100.0,
            angle_deg: 0.0,
            armor_thickness_mm: 40.0,
            shell: spec.shell,
            outcome: ImpactOutcome::Penetration { effective_mm: 40.0 },
        });
        app.update();
        assert!(!app.world().resource::<XRayHitReport>().active);
        assert!(app.world().resource::<DamageFeed>().items.is_empty());
        assert!(
            app.world().get::<TankCombat>(enemy).unwrap().alive,
            "враг должен остаться жив после нелетального попадания союзного ИИ"
        );

        // Игрок бьёт в боеукладку (2 попадания гарантируют переход Intact -> Damaged -> Destroyed)
        // -> XRayHitReport активен, DamageFeed заполнен, killer == Some(human).
        let mut reader = app
            .world_mut()
            .resource::<Messages<TankDestroyedMessage>>()
            .get_cursor();
        for _ in 0..2 {
            app.world_mut().write_message(ArmorImpactMessage {
                plate_entity: enemy,
                owner_tank: Some(enemy),
                zone: Some(ArmorZone::HullSideLeft),
                target_name: "Враг (борт)".into(),
                shooter: Some(human),
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

        assert!(app.world().resource::<XRayHitReport>().active);
        assert!(!app.world().resource::<DamageFeed>().items.is_empty());
        let msgs = app.world().resource::<Messages<TankDestroyedMessage>>();
        let killed: Vec<_> = reader.read(msgs).collect();
        assert_eq!(killed.len(), 1);
        assert_eq!(killed[0].killer, Some(human));
    }
}
