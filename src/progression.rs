//! Экономика, дерево исследований, боевые награды и сохранение прогресса игрока.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::ballistics::{ArmorImpactMessage, penetration::ImpactOutcome};
use crate::core::{BattleStatus, DamageRulesMode, GameState, SelectedMap, Team};
use crate::damage::{TankDestroyedMessage, TankHealth};
use crate::maps::RANDOM_MAP_ID;
use crate::tank::{PlayerTank, TankCombat, TankSpec};

/// Ключ сохранения прогресса в браузерном `localStorage` (WASM).
pub const SAVE_STORAGE_KEY: &str = "cerium_turbo_save_v1";

/// Стартовые резервные танки I уровня, доступные игроку с начала игры.
pub const STARTER_TANKS: [&str; 3] = ["ms_1", "pz_2_c", "m2a4"];

fn default_credits() -> u64 {
    1500
}

fn default_xp() -> u64 {
    350
}

/// Постоянный профиль прогресса игрока (кредиты, опыт, открытая и купленная техника).
#[derive(Resource, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlayerProgression {
    #[serde(default = "default_credits")]
    pub credits: u64,
    #[serde(default = "default_xp")]
    pub xp: u64,
    #[serde(default)]
    pub researched: HashSet<String>,
    #[serde(default)]
    pub owned: HashSet<String>,
    #[serde(default)]
    pub battles_played: u32,
    #[serde(default)]
    pub total_kills: u32,
    #[serde(default)]
    pub total_penetrations: u32,
}

impl Default for PlayerProgression {
    fn default() -> Self {
        let mut prog = Self {
            credits: default_credits(),
            xp: default_xp(),
            researched: HashSet::new(),
            owned: HashSet::new(),
            battles_played: 0,
            total_kills: 0,
            total_penetrations: 0,
        };
        prog.ensure_starters();
        prog
    }
}

impl PlayerProgression {
    /// Гарантирует, что стартовые танки `"ms_1"`, `"pz_2_c"` и `"m2a4"` всегда исследованы и куплены.
    pub fn ensure_starters(&mut self) {
        self.researched.remove("pz_1");
        self.owned.remove("pz_1");
        for id in STARTER_TANKS {
            if !self.researched.contains(id) {
                self.researched.insert(id.to_string());
            }
            if !self.owned.contains(id) {
                self.owned.insert(id.to_string());
            }
        }
    }

    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::to_string(self)
    }

    pub fn from_ron(s: &str) -> Result<Self, ron::error::SpannedError> {
        let mut prog: Self = ron::from_str(s)?;
        prog.ensure_starters();
        Ok(prog)
    }
}

/// Гарантирует наличие всех стартовых танков в профиле прогресса.
pub fn ensure_starters(prog: &mut PlayerProgression) {
    prog.ensure_starters();
}

/// Накопленные боевые показатели игрока за текущий бой.
#[derive(Resource, Debug, Clone, Default)]
pub struct BattleRewardTracker {
    pub penetrations: u32,
    pub damage_dealt: f32,
    pub kills: u32,
    pub random_map: bool,
    pub finalized: bool,
}

impl BattleRewardTracker {
    pub fn reset(&mut self, random_map: bool) {
        self.penetrations = 0;
        self.damage_dealt = 0.0;
        self.kills = 0;
        self.random_map = random_map;
        self.finalized = false;
    }
}

/// Итоговая сводка наград последнего завершённого боя для отображения в ангаре.
#[derive(Resource, Debug, Clone, Default)]
pub struct LastBattleReward {
    pub active: bool,
    pub victory: bool,
    pub early_exit: bool,
    pub random_bonus: bool,
    pub credits_earned: u64,
    pub xp_earned: u64,
    pub kills: u32,
    pub penetrations: u32,
    pub damage: f32,
}

pub fn is_owned(prog: &PlayerProgression, id: &str) -> bool {
    prog.owned.contains(id)
}

pub fn is_researched(prog: &PlayerProgression, id: &str) -> bool {
    prog.researched.contains(id)
}

/// Проверяет, исследован ли хотя бы один из пререквизитов танка (основной или альтернативный).
pub fn has_prerequisite_researched(prog: &PlayerProgression, spec: &TankSpec) -> bool {
    match (&spec.prerequisite, &spec.alt_prerequisite) {
        (None, None) => true,
        (Some(p), None) => is_researched(prog, p),
        (None, Some(alt)) => is_researched(prog, alt),
        (Some(p), Some(alt)) => is_researched(prog, p) || is_researched(prog, alt),
    }
}

pub fn can_research(prog: &PlayerProgression, spec: &TankSpec) -> bool {
    if is_researched(prog, &spec.id) {
        return false;
    }
    if !has_prerequisite_researched(prog, spec) {
        return false;
    }
    prog.xp >= spec.xp_cost as u64
}

pub fn can_buy(prog: &PlayerProgression, spec: &TankSpec) -> bool {
    is_researched(prog, &spec.id)
        && !is_owned(prog, &spec.id)
        && prog.credits >= spec.credit_cost as u64
}

pub fn research_tank(prog: &mut PlayerProgression, spec: &TankSpec) -> Result<(), &'static str> {
    if is_researched(prog, &spec.id) {
        return Err("Танк уже исследован");
    }
    if !has_prerequisite_researched(prog, spec) {
        return Err("Не исследован предыдущий танк");
    }
    let cost = spec.xp_cost as u64;
    if prog.xp < cost {
        return Err("Недостаточно опыта для исследования");
    }
    prog.xp -= cost;
    prog.researched.insert(spec.id.clone());
    save_progression(prog);
    Ok(())
}

pub fn buy_tank(prog: &mut PlayerProgression, spec: &TankSpec) -> Result<(), &'static str> {
    if !is_researched(prog, &spec.id) {
        return Err("Сначала необходимо исследовать танк");
    }
    if is_owned(prog, &spec.id) {
        return Err("Танк уже находится в ангаре");
    }
    let cost = spec.credit_cost as u64;
    if prog.credits < cost {
        return Err("Недостаточно серебра для покупки");
    }
    prog.credits -= cost;
    prog.owned.insert(spec.id.clone());
    save_progression(prog);
    Ok(())
}

pub fn save_progression(prog: &PlayerProgression) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten())
            && let Ok(serialized) = prog.to_ron()
        {
            let _ = storage.set_item(SAVE_STORAGE_KEY, &serialized);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = prog;
    }
}

pub fn load_progression() -> PlayerProgression {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten())
            && let Ok(Some(raw)) = storage.get_item(SAVE_STORAGE_KEY)
            && let Ok(prog) = PlayerProgression::from_ron(&raw)
        {
            return prog;
        }
    }
    let mut prog = PlayerProgression::default();
    prog.ensure_starters();
    prog
}

/// Подводит итог боя, начисляет серебро/опыт и сохраняет профиль (идемпотентно).
pub fn finalize_battle_reward(
    prog: &mut PlayerProgression,
    tracker: &mut BattleRewardTracker,
    last: &mut LastBattleReward,
    victory: bool,
    early_exit: bool,
) -> bool {
    if tracker.finalized {
        return false;
    }
    tracker.finalized = true;

    let dmg_u64 = tracker.damage_dealt.max(0.0).round() as u64;
    let action_credits =
        tracker.penetrations as u64 * 120 + dmg_u64 * 2 + tracker.kills as u64 * 450;
    let action_xp =
        tracker.penetrations as u64 * 40 + (dmg_u64 / 3) + tracker.kills as u64 * 180;

    let (match_credits, match_xp) = if victory {
        (900u64, 350u64)
    } else if !early_exit {
        (250u64, 100u64)
    } else {
        (0u64, 0u64)
    };

    let mut credits_earned = action_credits + match_credits;
    let mut xp_earned = action_xp + match_xp;

    let random_bonus = tracker.random_map && (credits_earned > 0 || xp_earned > 0);
    if tracker.random_map {
        if credits_earned > 0 {
            credits_earned = (credits_earned * 11 + 5) / 10;
        }
        if xp_earned > 0 {
            xp_earned = (xp_earned * 11 + 5) / 10;
        }
    }

    prog.credits = prog.credits.saturating_add(credits_earned);
    prog.xp = prog.xp.saturating_add(xp_earned);
    prog.battles_played = prog.battles_played.saturating_add(1);
    prog.total_kills = prog.total_kills.saturating_add(tracker.kills);
    prog.total_penetrations = prog.total_penetrations.saturating_add(tracker.penetrations);

    *last = LastBattleReward {
        active: true,
        victory,
        early_exit,
        random_bonus,
        credits_earned,
        xp_earned,
        kills: tracker.kills,
        penetrations: tracker.penetrations,
        damage: tracker.damage_dealt,
    };

    save_progression(prog);
    true
}

pub struct ProgressionPlugin;

impl Plugin for ProgressionPlugin {
    fn build(&self, app: &mut App) {
        let prog = load_progression();
        app.insert_resource(prog)
            .init_resource::<BattleRewardTracker>()
            .init_resource::<LastBattleReward>()
            .add_systems(OnEnter(GameState::InBattle), reset_battle_tracker)
            .add_systems(
                Update,
                track_battle_progression
                    .after(crate::damage::update_battle_status)
                    .run_if(in_state(GameState::InBattle)),
            )
            .add_systems(OnExit(GameState::InBattle), finalize_on_exit_battle);
    }
}

fn reset_battle_tracker(
    selected_map: Option<Res<SelectedMap>>,
    mut tracker: ResMut<BattleRewardTracker>,
    mut last: ResMut<LastBattleReward>,
) {
    let is_random = selected_map.is_some_and(|m| m.0 == RANDOM_MAP_ID);
    tracker.reset(is_random);
    *last = LastBattleReward::default();
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn track_battle_progression(
    mode: Option<Res<DamageRulesMode>>,
    status: Option<Res<BattleStatus>>,
    player_q: Query<Entity, With<PlayerTank>>,
    tanks: Query<(Entity, &TankCombat, Option<&TankHealth>)>,
    mut impacts: MessageReader<ArmorImpactMessage>,
    mut destroyed_r: MessageReader<TankDestroyedMessage>,
    mut prog: ResMut<PlayerProgression>,
    mut tracker: ResMut<BattleRewardTracker>,
    mut last: ResMut<LastBattleReward>,
    mut prev_tanks: Local<HashMap<Entity, (bool, f32)>>,
) {
    prev_tanks.retain(|e, _| tanks.contains(*e));

    if tracker.finalized {
        for _ in destroyed_r.read() {}
        for _ in impacts.read() {}
        for (ent, combat, health_opt) in &tanks {
            prev_tanks.insert(ent, (combat.alive, health_opt.map_or(0.0, |h| h.hp)));
        }
        return;
    }

    let human_ent = player_q.single().ok();
    let rules_mode = mode.as_deref().copied().unwrap_or_default();

    let mut destroyed_this_frame: HashSet<Entity> = HashSet::new();
    for msg in destroyed_r.read() {
        destroyed_this_frame.insert(msg.tank);
        if let Some(human) = human_ent
            && msg.killer == Some(human)
            && msg.team == Team::Enemy
        {
            tracker.kills = tracker.kills.saturating_add(1);
        }
    }

    if let Some(human) = human_ent {
        for msg in impacts.read() {
            if msg.shooter != Some(human) {
                continue;
            }
            if !matches!(msg.outcome, ImpactOutcome::Penetration { .. }) {
                continue;
            }
            let Some(tank_ent) = msg.owner_tank else {
                continue;
            };
            let Ok((_ent, combat, health_opt)) = tanks.get(tank_ent) else {
                continue;
            };
            if combat.team != Team::Enemy {
                continue;
            }

            let (was_alive, hp_before) = match prev_tanks.get(&tank_ent) {
                Some(&(alive, hp)) => (alive, hp),
                None => {
                    let alive = combat.alive || destroyed_this_frame.contains(&tank_ent);
                    let hp = health_opt.map_or(0.0, |h| h.max_hp);
                    (alive, hp)
                }
            };
            if !was_alive {
                continue;
            }

            tracker.penetrations = tracker.penetrations.saturating_add(1);
            let dmg = match rules_mode {
                DamageRulesMode::ArcadeHpAndModules => match health_opt {
                    Some(_) => msg.shell.arcade_damage.min(hp_before.max(0.0)),
                    None => msg.shell.arcade_damage,
                },
                DamageRulesMode::ModulesOnlyWt | DamageRulesMode::PrototypeTargets => {
                    130.0 + msg.shell.caliber_mm
                }
            };
            tracker.damage_dealt += dmg;

            if let Some(entry) = prev_tanks.get_mut(&tank_ent)
                && rules_mode == DamageRulesMode::ArcadeHpAndModules
            {
                entry.1 = (entry.1 - dmg).max(0.0);
                if entry.1 <= 0.0 {
                    entry.0 = false;
                }
            }
        }
    } else {
        for _ in impacts.read() {}
    }

    for (ent, combat, health_opt) in &tanks {
        prev_tanks.insert(ent, (combat.alive, health_opt.map_or(0.0, |h| h.hp)));
    }

    if let Some(battle_status) = status
        && *battle_status != BattleStatus::Ongoing
        && !tracker.finalized
    {
        let victory = *battle_status == BattleStatus::Victory;
        finalize_battle_reward(&mut prog, &mut tracker, &mut last, victory, false);
    }
}

fn finalize_on_exit_battle(
    status: Option<Res<BattleStatus>>,
    mut prog: ResMut<PlayerProgression>,
    mut tracker: ResMut<BattleRewardTracker>,
    mut last: ResMut<LastBattleReward>,
) {
    if !tracker.finalized {
        let current = status.map_or(BattleStatus::Ongoing, |s| *s);
        let victory = current == BattleStatus::Victory;
        let early_exit = current == BattleStatus::Ongoing;
        finalize_battle_reward(&mut prog, &mut tracker, &mut last, victory, early_exit);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::ballistics::{HitLog, ShellSpec};
    use crate::damage::{DamagePlugin, TankCrew, TankModules};
    use crate::tank::{ArmorZone, TankRoot, available_tanks, load_tank_spec};
    use bevy::state::app::StatesPlugin;
    use bevy::time::TimeUpdateStrategy;

    fn sample_spec(
        id: &str,
        xp_cost: u32,
        credit_cost: u32,
        prerequisite: Option<&str>,
    ) -> TankSpec {
        let mut spec = load_tank_spec("t34_85")
            .or_else(|| available_tanks().into_iter().next())
            .expect("built-in tank spec must exist");
        spec.id = id.to_string();
        spec.xp_cost = xp_cost;
        spec.credit_cost = credit_cost;
        spec.prerequisite = prerequisite.map(str::to_string);
        spec
    }

    #[test]
    fn starters_are_always_researched_and_owned() {
        let mut prog = PlayerProgression::default();
        assert_eq!(prog.credits, 1500);
        assert_eq!(prog.xp, 350);
        assert_eq!(prog.battles_played, 0);
        for starter in STARTER_TANKS {
            assert!(is_researched(&prog, starter), "{starter} must be researched");
            assert!(is_owned(&prog, starter), "{starter} must be owned");
        }

        prog.researched.remove("ms_1");
        prog.owned.remove("pz_2_c");
        ensure_starters(&mut prog);
        assert!(is_researched(&prog, "ms_1"));
        assert!(is_owned(&prog, "pz_2_c"));
    }

    #[test]
    fn research_and_buy_enforce_prerequisites_and_deduct_costs() {
        let mut prog = PlayerProgression::default();
        let tier2 = sample_spec("test_tier2", 200, 1000, Some("ms_1"));
        let tier3 = sample_spec("test_tier3", 300, 2000, Some("test_tier2"));

        assert!(can_research(&prog, &tier2));
        assert!(!can_research(&prog, &tier3));
        assert!(!can_buy(&prog, &tier2));

        // Нельзя исследовать следующий танк без предыдущего.
        assert!(research_tank(&mut prog, &tier3).is_err());
        assert_eq!(prog.xp, 350);

        // Исследуем tier2 -> опыт уменьшается, но танк ещё не куплен.
        assert!(research_tank(&mut prog, &tier2).is_ok());
        assert_eq!(prog.xp, 150);
        assert!(is_researched(&prog, "test_tier2"));
        assert!(!is_owned(&prog, "test_tier2"));
        assert!(research_tank(&mut prog, &tier2).is_err());

        // Пререквизит tier3 выполнен, но не хватает опыта (150 < 300).
        assert!(!can_research(&prog, &tier3));
        assert!(research_tank(&mut prog, &tier3).is_err());

        // Покупаем tier2 за серебро (1500 >= 1000).
        assert!(can_buy(&prog, &tier2));
        assert!(buy_tank(&mut prog, &tier2).is_ok());
        assert_eq!(prog.credits, 500);
        assert!(is_owned(&prog, "test_tier2"));
        assert!(!can_buy(&prog, &tier2));
        assert!(buy_tank(&mut prog, &tier2).is_err());

        // Недостаточно серебра для покупки второго исследованного танка.
        prog.xp = 500;
        assert!(research_tank(&mut prog, &tier3).is_ok());
        assert!(!can_buy(&prog, &tier3));
        assert!(buy_tank(&mut prog, &tier3).is_err());

        // Альтернативные ветки (alt_prerequisite): достаточно исследовать ОДИН из пререквизитов.
        let mut alt_prog = PlayerProgression {
            xp: 40_000,
            credits: 80_000,
            ..Default::default()
        };
        let pz4a = load_tank_spec("pz_4_a").unwrap();
        let pz4 = load_tank_spec("pz_4").unwrap();
        let panther = load_tank_spec("panther").unwrap();
        let tiger2 = load_tank_spec("tiger_2").unwrap();
        assert!(research_tank(&mut alt_prog, &pz4a).is_ok());
        // Исследован только Pz IV ausf A (без Pz III ausf E), но Pz IV ausf H уже открыт.
        assert!(can_research(&alt_prog, &pz4));
        assert!(research_tank(&mut alt_prog, &pz4).is_ok());
        assert!(research_tank(&mut alt_prog, &panther).is_ok());
        // Исследована только Panther (без Tiger I), но Tiger II на V ранге уже доступен к исследованию.
        assert!(can_research(&alt_prog, &tiger2));
        assert!(research_tank(&mut alt_prog, &tiger2).is_ok());

        let m3 = load_tank_spec("m3_stuart").unwrap();
        let m8a1 = load_tank_spec("m8a1").unwrap();
        let m4a1 = load_tank_spec("m4a1_75").unwrap();
        assert!(research_tank(&mut alt_prog, &m3).is_ok());
        assert!(research_tank(&mut alt_prog, &m8a1).is_ok());
        assert!(can_research(&alt_prog, &m4a1));
    }

    #[test]
    fn ron_roundtrip_preserves_progression_and_restores_starters() {
        let mut prog = PlayerProgression {
            credits: 4200,
            xp: 980,
            battles_played: 5,
            total_kills: 9,
            total_penetrations: 22,
            ..Default::default()
        };
        prog.researched.insert("t34_85".into());
        prog.owned.insert("t34_85".into());

        let ron_str = prog.to_ron().expect("serialize progression");
        let parsed = PlayerProgression::from_ron(&ron_str).expect("deserialize progression");
        assert_eq!(parsed, prog);

        // Даже если в сохранении отсутствовали стартеры, from_ron восстановит их.
        let mut stripped = prog.clone();
        stripped.researched.remove("m2a4");
        stripped.owned.remove("ms_1");
        let stripped_ron = stripped.to_ron().expect("serialize stripped");
        let restored = PlayerProgression::from_ron(&stripped_ron).expect("deserialize stripped");
        for starter in STARTER_TANKS {
            assert!(is_researched(&restored, starter));
            assert!(is_owned(&restored, starter));
        }

        let native_loaded = load_progression();
        assert!(is_owned(&native_loaded, "ms_1"));
    }

    #[test]
    fn finalize_battle_reward_handles_outcomes_bonuses_and_idempotency() {
        // 1. Победа на обычной карте.
        let mut prog = PlayerProgression::default();
        let mut tracker = BattleRewardTracker::default();
        let mut last = LastBattleReward::default();
        tracker.reset(false);
        tracker.penetrations = 3;
        tracker.damage_dealt = 540.0;
        tracker.kills = 1;

        assert!(finalize_battle_reward(&mut prog, &mut tracker, &mut last, true, false));
        // Действия: 3*120 + 540*2 + 1*450 = 1890 кр., 3*40 + 540/3 + 1*180 = 480 опыта.
        // Победа: +900 кр., +350 опыта -> 2790 кр., 830 опыта.
        assert_eq!(last.credits_earned, 2790);
        assert_eq!(last.xp_earned, 830);
        assert!(last.active && last.victory && !last.early_exit && !last.random_bonus);
        assert_eq!(prog.credits, 1500 + 2790);
        assert_eq!(prog.xp, 350 + 830);
        assert_eq!(prog.battles_played, 1);
        assert_eq!(prog.total_kills, 1);
        assert_eq!(prog.total_penetrations, 3);

        // Идемпотентность: повторный вызов не меняет профиль.
        assert!(!finalize_battle_reward(&mut prog, &mut tracker, &mut last, true, false));
        assert_eq!(prog.credits, 1500 + 2790);

        // 2. Победа со случайной картой (+10% с округлением).
        tracker.reset(true);
        tracker.penetrations = 3;
        tracker.damage_dealt = 540.0;
        tracker.kills = 1;
        assert!(finalize_battle_reward(&mut prog, &mut tracker, &mut last, true, false));
        assert!(last.random_bonus);
        assert_eq!(last.credits_earned, (2790 * 11 + 5) / 10);
        assert_eq!(last.xp_earned, (830 * 11 + 5) / 10);

        // 3. Поражение без досрочного выхода -> утешительные (250, 100).
        tracker.reset(false);
        assert!(finalize_battle_reward(&mut prog, &mut tracker, &mut last, false, false));
        assert_eq!(last.credits_earned, 250);
        assert_eq!(last.xp_earned, 100);
        assert!(!last.victory && !last.early_exit);

        // 4. Нулевой ранний выход [Tab] на случайной карте -> строго 0 наград (анти-эксплойт).
        let credits_before = prog.credits;
        let xp_before = prog.xp;
        tracker.reset(true);
        assert!(finalize_battle_reward(&mut prog, &mut tracker, &mut last, false, true));
        assert_eq!(last.credits_earned, 0);
        assert_eq!(last.xp_earned, 0);
        assert!(last.early_exit && !last.random_bonus);
        assert_eq!(prog.credits, credits_before);
        assert_eq!(prog.xp, xp_before);

        // 5. Досрочный выход после результативных попаданий -> награда только за действия.
        tracker.reset(false);
        tracker.penetrations = 2;
        tracker.damage_dealt = 360.0;
        assert!(finalize_battle_reward(&mut prog, &mut tracker, &mut last, false, true));
        assert_eq!(last.credits_earned, 2 * 120 + 360 * 2);
        assert_eq!(last.xp_earned, 2 * 40 + 360 / 3);
    }

    #[test]
    fn progression_plugin_tracks_human_hits_kills_and_auto_finalizes() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin, StatesPlugin))
            .init_state::<GameState>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<HitLog>()
            .init_resource::<BattleStatus>()
            .insert_resource(DamageRulesMode::ArcadeHpAndModules)
            .insert_resource(SelectedMap(RANDOM_MAP_ID.into()))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                0.25,
            )))
            .add_message::<ArmorImpactMessage>()
            .add_plugins((DamagePlugin, ProgressionPlugin));

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
        let enemy_1 = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
                TankHealth::new(500.0),
                TankModules::for_spec(&spec),
                TankCombat::new(Team::Enemy, "Враг 1".into()),
            ))
            .id();
        let enemy_2 = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
                TankHealth::new(100.0),
                TankModules::for_spec(&spec),
                TankCombat::new(Team::Enemy, "Враг 2".into()),
            ))
            .id();

        app.world_mut()
            .resource_mut::<NextState<GameState>>()
            .set(GameState::InBattle);
        app.finish();
        app.cleanup();
        app.update();

        let fixed_shell = ShellSpec {
            arcade_damage: 180.0,
            ..spec.shell
        };

        // 1. Союзный ИИ пробивает Врага 1 -> НЕ засчитывается игроку.
        app.world_mut().write_message(ArmorImpactMessage {
            plate_entity: enemy_1,
            owner_tank: Some(enemy_1),
            zone: Some(ArmorZone::HullRear),
            target_name: "Враг 1".into(),
            shooter: Some(ally_ai),
            shooter_team: Some(Team::Player),
            hit_point: Vec3::new(0.0, 0.72, 2.8),
            hit_normal: Vec3::Z,
            ray_dir: Vec3::NEG_Z,
            dist_m: 100.0,
            angle_deg: 0.0,
            armor_thickness_mm: 40.0,
            shell: fixed_shell,
            outcome: ImpactOutcome::Penetration { effective_mm: 40.0 },
        });
        app.update();
        {
            let tr = app.world().resource::<BattleRewardTracker>();
            assert_eq!(tr.penetrations, 0);
            assert_eq!(tr.damage_dealt, 0.0);
        }

        // 2. Рикошет игрока не засчитывается, а пробитие Врага 2 (100 HP) даёт ровно 100 урона + 1 фраг.
        app.world_mut().write_message(ArmorImpactMessage {
            plate_entity: enemy_1,
            owner_tank: Some(enemy_1),
            zone: Some(ArmorZone::HullFrontUpper),
            target_name: "Враг 1".into(),
            shooter: Some(human),
            shooter_team: Some(Team::Player),
            hit_point: Vec3::ZERO,
            hit_normal: Vec3::NEG_Z,
            ray_dir: Vec3::Z,
            dist_m: 100.0,
            angle_deg: 75.0,
            armor_thickness_mm: 45.0,
            shell: fixed_shell,
            outcome: ImpactOutcome::Ricochet,
        });
        app.world_mut().write_message(ArmorImpactMessage {
            plate_entity: enemy_2,
            owner_tank: Some(enemy_2),
            zone: Some(ArmorZone::HullRear),
            target_name: "Враг 2".into(),
            shooter: Some(human),
            shooter_team: Some(Team::Player),
            hit_point: Vec3::new(0.0, 0.72, 2.8),
            hit_normal: Vec3::Z,
            ray_dir: Vec3::NEG_Z,
            dist_m: 100.0,
            angle_deg: 0.0,
            armor_thickness_mm: 40.0,
            shell: fixed_shell,
            outcome: ImpactOutcome::Penetration { effective_mm: 40.0 },
        });
        app.update();
        {
            let tr = app.world().resource::<BattleRewardTracker>();
            assert_eq!(tr.penetrations, 1);
            assert!((tr.damage_dealt - 100.0).abs() < 1e-2, "dmg={}", tr.damage_dealt);
            assert_eq!(tr.kills, 1);
            assert!(!tr.finalized);
        }

        // 3. Повторное попадание по уже уничтоженному Врагу 2 не начисляет наград.
        app.world_mut().write_message(ArmorImpactMessage {
            plate_entity: enemy_2,
            owner_tank: Some(enemy_2),
            zone: Some(ArmorZone::HullRear),
            target_name: "Враг 2".into(),
            shooter: Some(human),
            shooter_team: Some(Team::Player),
            hit_point: Vec3::new(0.0, 0.72, 2.8),
            hit_normal: Vec3::Z,
            ray_dir: Vec3::NEG_Z,
            dist_m: 100.0,
            angle_deg: 0.0,
            armor_thickness_mm: 40.0,
            shell: fixed_shell,
            outcome: ImpactOutcome::Penetration { effective_mm: 40.0 },
        });
        app.update();
        assert_eq!(app.world().resource::<BattleRewardTracker>().penetrations, 1);

        // 4. Союзный ИИ добивает последнего Врага 1 -> Победа и авто-финализация!
        for _ in 0..2 {
            app.world_mut().write_message(ArmorImpactMessage {
                plate_entity: enemy_1,
                owner_tank: Some(enemy_1),
                zone: Some(ArmorZone::HullRear),
                target_name: "Враг 1".into(),
                shooter: Some(ally_ai),
                shooter_team: Some(Team::Player),
                hit_point: Vec3::new(0.0, 0.72, 2.8),
                hit_normal: Vec3::Z,
                ray_dir: Vec3::NEG_Z,
                dist_m: 100.0,
                angle_deg: 0.0,
                armor_thickness_mm: 40.0,
                shell: fixed_shell,
                outcome: ImpactOutcome::Penetration { effective_mm: 40.0 },
            });
            app.update();
        }

        assert_eq!(*app.world().resource::<BattleStatus>(), BattleStatus::Victory);
        let tr = app.world().resource::<BattleRewardTracker>();
        assert!(tr.finalized);
        assert_eq!(tr.kills, 1);
        assert_eq!(tr.penetrations, 1);

        let last = app.world().resource::<LastBattleReward>();
        assert!(last.active && last.victory && last.random_bonus);
        // 1*120 + 100*2 + 1*450 + 900 = 1670 -> * 1.1 = (1670*11 + 5)/10 = 1837.
        assert_eq!(last.credits_earned, 1837);
        assert_eq!(app.world().resource::<PlayerProgression>().credits, 1500 + 1837);
    }

    #[test]
    fn wt_mode_awards_caliber_scaled_damage_on_penetration() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin, StatesPlugin))
            .init_state::<GameState>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<HitLog>()
            .init_resource::<BattleStatus>()
            .insert_resource(DamageRulesMode::ModulesOnlyWt)
            .insert_resource(SelectedMap("proving_grounds".into()))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                0.25,
            )))
            .add_message::<ArmorImpactMessage>()
            .add_plugins((DamagePlugin, ProgressionPlugin));

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
                TankCombat::new(Team::Enemy, "Враг".into()),
            ))
            .id();

        app.world_mut()
            .resource_mut::<NextState<GameState>>()
            .set(GameState::InBattle);
        app.finish();
        app.cleanup();
        app.update();

        app.world_mut().write_message(ArmorImpactMessage {
            plate_entity: enemy,
            owner_tank: Some(enemy),
            zone: Some(ArmorZone::HullRear),
            target_name: "Враг".into(),
            shooter: Some(human),
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

        let tr = app.world().resource::<BattleRewardTracker>();
        assert_eq!(tr.penetrations, 1);
        assert!(
            (tr.damage_dealt - (130.0 + spec.shell.caliber_mm)).abs() < 1e-2,
            "wt dmg={}",
            tr.damage_dealt
        );
    }
}
