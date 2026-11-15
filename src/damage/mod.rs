//! Система повреждений: маршрутизация режимов `DamageRulesMode`.
//!
//! - Этапы 1–2: `PrototypeTargets` (пробития по счётчику, без общего ХП и экипажа).
//! - Этап 3+: `ArcadeHpAndModules` (общее ХП + ХП модулей, стиль WoT).
//! - Этап 6: `ModulesOnlyWt` (только модули и экипаж без общего ХП, стиль War Thunder).

pub mod crew;
pub mod hp_model;
pub mod modules;
pub mod spall;
pub mod wt_model;

use bevy::prelude::*;

use crate::ballistics::{ArmorImpactMessage, HitKind, HitLog, penetration::ImpactOutcome};
use crate::core::{BattleStatus, DamageRulesMode, GameState, Team};
use crate::tank::TankCombat;
use crate::voxel::{Material, Rng, VoxelAssets, spawn_debris_burst};

pub use crew::{CrewMember, CrewRole, CrewState, TankCrew};
pub use hp_model::{
    DamageFeed, PenIndicator, TankHealth, classify_penetration, destroy_tank,
};
pub use modules::{ModuleKind, ModuleState, TankModules};
pub use spall::{compute_residual_penetration, generate_spall_rays, spall_cone_params};
pub use wt_model::XRayHitReport;

/// Событие уничтожения танка (для детонации БК, сброса башни, дыма и звука).
#[derive(Message, Debug, Clone)]
pub struct TankDestroyedMessage {
    pub tank: Entity,
    pub pos: Vec3,
    pub team: Team,
    pub ammo_detonated: bool,
    pub killer: Option<Entity>,
}

/// Действия с пожаром и ремонтом модулей (для частиц пены/огня и звука).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TankActionKind {
    FireStarted,
    FireExtinguished,
    RepairUsed,
    AutoRepaired,
}

#[derive(Message, Debug, Clone)]
pub struct TankActionMessage {
    pub tank: Entity,
    pub pos: Vec3,
    pub is_player: bool,
    pub kind: TankActionKind,
}

pub struct DamagePlugin;

impl Plugin for DamagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DamageFeed>()
            .init_resource::<XRayHitReport>()
            .add_message::<TankDestroyedMessage>()
            .add_message::<TankActionMessage>()
            .add_systems(
                Update,
                (
                    handle_prototype_impacts,
                    hp_model::handle_arcade_hp_impacts,
                    wt_model::handle_wt_modules_impacts,
                    hp_model::tick_modules_repair_and_fire,
                    wt_model::tick_wt_crew_and_fire,
                    update_battle_status,
                )
                    .chain()
                    .run_if(in_state(GameState::InBattle)),
            );
    }
}

/// Этап 2: обработка попаданий в режиме `PrototypeTargets` (без общего ХП).
#[allow(clippy::too_many_arguments)]
fn handle_prototype_impacts(
    mut commands: Commands,
    mode: Res<DamageRulesMode>,
    mut impacts: MessageReader<ArmorImpactMessage>,
    mut tanks: Query<(&GlobalTransform, &mut TankCombat)>,
    mut log: ResMut<HitLog>,
    assets: Option<Res<VoxelAssets>>,
    mut destroyed_w: MessageWriter<TankDestroyedMessage>,
    mut seed: Local<u64>,
) {
    if *mode != DamageRulesMode::PrototypeTargets {
        return;
    }

    for msg in impacts.read() {
        let Some(tank_ent) = msg.owner_tank else {
            continue;
        };
        let Ok((gt, mut combat)) = tanks.get_mut(tank_ent) else {
            continue;
        };
        if !combat.alive {
            continue;
        }
        // Не засчитываем дружественный огонь.
        if msg.shooter_team == Some(combat.team) {
            continue;
        }
        if !matches!(msg.outcome, ImpactOutcome::Penetration { .. }) {
            continue;
        }

        combat.prototype_hits = combat.prototype_hits.saturating_add(1);
        let zone = msg.zone.map_or("броня", |z| z.ru_name());

        if combat.prototype_hits >= combat.prototype_max_hits {
            combat.alive = false;
            combat.immobilized = true;
            combat.gun_jammed = true;
            log.push(
                HitKind::Penetration,
                format!(
                    "УНИЧТОЖЕН: {} (попадание в {}, {}/{})",
                    combat.tank_name, zone, combat.prototype_hits, combat.prototype_max_hits
                ),
            );
            destroyed_w.write(TankDestroyedMessage {
                tank: tank_ent,
                pos: gt.translation(),
                team: combat.team,
                ammo_detonated: false,
                killer: msg.shooter,
            });
            if let Some(assets) = &assets {
                *seed = seed.wrapping_add(0xD1B5_4A32_D192_ED03);
                let mut rng = Rng::new(*seed | 1);
                spawn_debris_burst(
                    &mut commands,
                    assets,
                    0,
                    gt.translation() + Vec3::Y * 1.1,
                    Vec3::Y,
                    Material::EnemyMark,
                    24,
                    0.45,
                    &mut rng,
                );
            }
        } else {
            log.push(
                HitKind::Penetration,
                format!(
                    "{}: пробитие {} ({}/{})",
                    combat.tank_name, zone, combat.prototype_hits, combat.prototype_max_hits
                ),
            );
        }
    }
}

/// Проверяет жив ли игрок и остались ли противники.
pub fn update_battle_status(
    mut status: ResMut<BattleStatus>,
    tanks: Query<&TankCombat>,
) {
    let mut player_total = 0usize;
    let mut player_alive = 0usize;
    let mut enemy_total = 0usize;
    let mut enemy_alive = 0usize;

    for c in &tanks {
        match c.team {
            Team::Player => {
                player_total += 1;
                if c.alive {
                    player_alive += 1;
                }
            }
            Team::Enemy => {
                enemy_total += 1;
                if c.alive {
                    enemy_alive += 1;
                }
            }
        }
    }

    if player_total > 0 && player_alive == 0 {
        *status = BattleStatus::Defeat;
    } else if enemy_total > 0 && enemy_alive == 0 {
        *status = BattleStatus::Victory;
    } else {
        *status = BattleStatus::Ongoing;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tank_combat_defaults_match_stage2_thresholds() {
        let enemy = TankCombat::new(Team::Enemy, "Т-34-85".into());
        assert!(enemy.alive);
        assert_eq!(enemy.prototype_max_hits, 3);

        let player = TankCombat::new(Team::Player, "Т-34-85".into());
        assert_eq!(player.prototype_max_hits, 5);
    }
}
