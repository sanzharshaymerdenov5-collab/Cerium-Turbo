//! Балансировщик боёв по уровню техники (±1 уровень от танка игрока).

use bevy::prelude::*;

use crate::tank::spec::TankSpec;

/// Итоговый состав 5v5 для текущего боя (союзники и противники в вилке ±1 ур.).
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct ActiveBattleLineup {
    pub player_tier: u8,
    pub min_tier: u8,
    pub max_tier: u8,
    pub ally_tank_ids: Vec<String>,
    pub enemy_tank_ids: Vec<String>,
}

impl ActiveBattleLineup {
    /// Возвращает диапазон допустимых уровней `(min_tier, max_tier)` для указанного уровня игрока.
    pub fn tier_window(player_tier: u8) -> (u8, u8) {
        let t = player_tier.clamp(1, 7);
        (t.saturating_sub(1).max(1), (t + 1).min(7))
    }
}

/// Минималистичный детерминированный ГПСЧ (SplitMix64).
struct Rng64(u64);

impl Rng64 {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn gen_range(&mut self, upper: usize) -> usize {
        if upper <= 1 {
            0
        } else {
            (self.next_u64() % upper as u64) as usize
        }
    }

    fn shuffle<T>(&mut self, slice: &mut [T]) {
        for i in (1..slice.len()).rev() {
            let j = self.gen_range(i + 1);
            slice.swap(i, j);
        }
    }
}

/// Подбирает состав союзников и противников strictly в пределах ±1 уровня от `player.tier`.
///
/// Гарантии:
/// - Каждый выбранный танк в `ally_tank_ids` и `enemy_tank_ids` имеет уровень в `[min_tier..=max_tier]`,
///   где `min_tier = (player.tier - 1).max(1)`, `max_tier = (player.tier + 1).min(7)`.
/// - При сопоставимом числе слотов (`player + allies == enemies`) суммарный уровень команд выравнивается.
/// - Внутри каждого уровня танки перемешиваются между нациями, избегая дублей при наличии выбора.
pub fn build_battle_lineup(
    player: &TankSpec,
    ally_slots: usize,
    enemy_slots: usize,
    pool: &[TankSpec],
    seed: u64,
) -> ActiveBattleLineup {
    let (min_tier, max_tier) = ActiveBattleLineup::tier_window(player.tier);

    let mut eligible: Vec<&TankSpec> = pool
        .iter()
        .filter(|t| t.tier >= min_tier && t.tier <= max_tier)
        .collect();
    if eligible.is_empty() && !pool.is_empty() {
        let best_tier = pool
            .iter()
            .min_by_key(|t| t.tier.abs_diff(player.tier))
            .map(|t| t.tier)
            .unwrap_or(player.tier);
        eligible = pool.iter().filter(|t| t.tier == best_tier).collect();
    }
    if eligible.is_empty() {
        return ActiveBattleLineup {
            player_tier: player.tier,
            min_tier,
            max_tier,
            ally_tank_ids: vec![player.id.clone(); ally_slots],
            enemy_tank_ids: vec![player.id.clone(); enemy_slots],
        };
    }

    let mut avail_tiers: Vec<u8> = eligible.iter().map(|t| t.tier).collect();
    avail_tiers.sort_unstable();
    avail_tiers.dedup();
    let min_avail = *avail_tiers.first().unwrap_or(&player.tier);
    let max_avail = *avail_tiers.last().unwrap_or(&player.tier);

    let mut rng = Rng64::new(seed ^ ((player.tier as u64) << 32));

    // Формируем уровни команды противника с разбросом по доступной вилке.
    let mut enemy_tiers = Vec::with_capacity(enemy_slots);
    if enemy_slots > 0 {
        let mut pattern = Vec::new();
        // Если есть 3 уровня (например III, IV, V) или 2 уровня — включаем микс, но всегда удерживаем
        // достижимость суммарного баланса со стороны союзников + игрока.
        for &t in &avail_tiers {
            pattern.push(t);
        }
        let anchor_tier = if avail_tiers.contains(&player.tier) {
            player.tier
        } else {
            min_avail
        };
        while pattern.len() < enemy_slots {
            let pick = match rng.gen_range(4) {
                0 => min_avail,
                1 | 2 => anchor_tier,
                _ => max_avail,
            };
            pattern.push(pick);
        }
        rng.shuffle(&mut pattern);
        enemy_tiers.extend(pattern.into_iter().take(enemy_slots));

        // Ограничиваем сумму уровней врагов, чтобы команда игрока (1 + ally_slots) могла её уравновесить.
        let min_possible_friend_sum = player.tier as i32 + (ally_slots as i32) * (min_avail as i32);
        let max_possible_friend_sum = player.tier as i32 + (ally_slots as i32) * (max_avail as i32);
        if ally_slots + 1 == enemy_slots {
            let mut enemy_sum: i32 = enemy_tiers.iter().map(|&t| t as i32).sum();
            let mut guard = 0;
            while enemy_sum > max_possible_friend_sum && guard < 32 {
                if let Some(pos) = enemy_tiers
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| **t > min_avail)
                    .max_by_key(|(_, t)| **t)
                    .map(|(i, _)| i)
                {
                    let cur = enemy_tiers[pos];
                    if let Some(&prev) = avail_tiers.iter().rev().find(|&&t| t < cur) {
                        enemy_tiers[pos] = prev;
                        enemy_sum -= (cur - prev) as i32;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
                guard += 1;
            }
            guard = 0;
            while enemy_sum < min_possible_friend_sum && guard < 32 {
                if let Some(pos) = enemy_tiers
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| **t < max_avail)
                    .min_by_key(|(_, t)| **t)
                    .map(|(i, _)| i)
                {
                    let cur = enemy_tiers[pos];
                    if let Some(&next) = avail_tiers.iter().find(|&&t| t > cur) {
                        enemy_tiers[pos] = next;
                        enemy_sum += (next - cur) as i32;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
                guard += 1;
            }
        }
        rng.shuffle(&mut enemy_tiers);
    }

    // Подбираем уровни союзников под суммарный баланс (player.tier + sum(ally) ≈ sum(enemy)).
    let mut ally_tiers = Vec::with_capacity(ally_slots);
    if ally_slots > 0 {
        let anchor_tier = if avail_tiers.contains(&player.tier) {
            player.tier
        } else {
            min_avail
        };
        if avail_tiers.len() >= 2 && ally_slots >= 2 {
            // Разнообразие на старте + затем подгонка к сумме врагов.
            for idx in 0..ally_slots {
                ally_tiers.push(avail_tiers[idx % avail_tiers.len()]);
            }
        } else {
            ally_tiers.resize(ally_slots, anchor_tier);
        }

        let target_ally_sum = if enemy_slots > 0 {
            let enemy_sum: i32 = enemy_tiers.iter().map(|&t| t as i32).sum();
            let avg_enemy = enemy_sum as f32 / enemy_slots as f32;
            let ideal_friend_total = (avg_enemy * (ally_slots + 1) as f32).round() as i32;
            ideal_friend_total - player.tier as i32
        } else {
            (ally_slots as i32) * (anchor_tier as i32)
        };

        let mut cur_sum: i32 = ally_tiers.iter().map(|&t| t as i32).sum();
        let mut guard = 0;
        while cur_sum < target_ally_sum && guard < 64 {
            let mut improved = false;
            if let Some(pos) = ally_tiers
                .iter()
                .enumerate()
                .filter(|(_, t)| **t < max_avail)
                .min_by_key(|(_, t)| **t)
                .map(|(i, _)| i)
            {
                let cur = ally_tiers[pos];
                if let Some(&next) = avail_tiers.iter().find(|&&t| t > cur) {
                    ally_tiers[pos] = next;
                    cur_sum += (next - cur) as i32;
                    improved = true;
                }
            }
            if !improved {
                break;
            }
            guard += 1;
        }
        guard = 0;
        while cur_sum > target_ally_sum && guard < 64 {
            let mut improved = false;
            if let Some(pos) = ally_tiers
                .iter()
                .enumerate()
                .filter(|(_, t)| **t > min_avail)
                .max_by_key(|(_, t)| **t)
                .map(|(i, _)| i)
            {
                let cur = ally_tiers[pos];
                if let Some(&prev) = avail_tiers.iter().rev().find(|&&t| t < cur) {
                    ally_tiers[pos] = prev;
                    cur_sum -= (cur - prev) as i32;
                    improved = true;
                }
            }
            if !improved {
                break;
            }
            guard += 1;
        }
        rng.shuffle(&mut ally_tiers);
    }

    let ally_tank_ids = pick_tanks_for_tiers(&ally_tiers, &eligible, &mut rng);
    let enemy_tank_ids = pick_tanks_for_tiers(&enemy_tiers, &eligible, &mut rng);

    ActiveBattleLineup {
        player_tier: player.tier,
        min_tier,
        max_tier,
        ally_tank_ids,
        enemy_tank_ids,
    }
}

fn pick_tanks_for_tiers(
    tiers: &[u8],
    eligible: &[&TankSpec],
    rng: &mut Rng64,
) -> Vec<String> {
    let mut chosen: Vec<String> = Vec::with_capacity(tiers.len());
    for &target_tier in tiers {
        let mut tier_candidates: Vec<&TankSpec> = eligible
            .iter()
            .copied()
            .filter(|t| t.tier == target_tier)
            .collect();
        if tier_candidates.is_empty() {
            tier_candidates = eligible.to_vec();
        }
        rng.shuffle(&mut tier_candidates);

        // Предпочитаем машины, которые ещё не встречались на этой стороне (макс. 1..2 дубля).
        tier_candidates.sort_by_key(|t| chosen.iter().filter(|id| *id == &t.id).count());
        let picked = tier_candidates
            .first()
            .map(|t| t.id.clone())
            .unwrap_or_else(|| "ms_1".to_string());
        chosen.push(picked);
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tank::spec::{available_tanks, load_tank_spec};

    #[test]
    fn matchmaker_enforces_plus_minus_one_tier_across_all_player_tanks() {
        let pool = available_tanks();
        assert_eq!(pool.len(), 30);

        for player in &pool {
            let (exp_min, exp_max) = ActiveBattleLineup::tier_window(player.tier);
            assert_eq!(exp_min, player.tier.saturating_sub(1).max(1));
            assert_eq!(exp_max, (player.tier + 1).min(7));

            for seed in [1_u64, 42, 999, 123_456] {
                let lineup = build_battle_lineup(player, 4, 5, &pool, seed);
                assert_eq!(lineup.ally_tank_ids.len(), 4);
                assert_eq!(lineup.enemy_tank_ids.len(), 5);
                assert_eq!(lineup.min_tier, exp_min);
                assert_eq!(lineup.max_tier, exp_max);

                let mut ally_sum = player.tier as i32;
                for aid in &lineup.ally_tank_ids {
                    let spec = load_tank_spec(aid).expect("ally tank_id должен загружаться");
                    assert!(
                        (exp_min..=exp_max).contains(&spec.tier),
                        "игрок {} (ур.{}) получил союзника {} (ур.{}) вне [{}..={}]",
                        player.id,
                        player.tier,
                        spec.id,
                        spec.tier,
                        exp_min,
                        exp_max
                    );
                    ally_sum += spec.tier as i32;
                }

                let mut enemy_sum = 0_i32;
                for eid in &lineup.enemy_tank_ids {
                    let spec = load_tank_spec(eid).expect("enemy tank_id должен загружаться");
                    assert!(
                        (exp_min..=exp_max).contains(&spec.tier),
                        "игрок {} (ур.{}) получил врага {} (ур.{}) вне [{}..={}]",
                        player.id,
                        player.tier,
                        spec.id,
                        spec.tier,
                        exp_min,
                        exp_max
                    );
                    enemy_sum += spec.tier as i32;
                }

                assert!(
                    (ally_sum - enemy_sum).abs() <= 2,
                    "дисбаланс уровней команд для {} (seed={seed}): allies={ally_sum} vs enemies={enemy_sum}",
                    player.id
                );
            }
        }
    }

    #[test]
    fn matchmaker_specific_tiers_stay_within_exact_windows() {
        let pool = available_tanks();
        let ms1 = load_tank_spec("ms_1").unwrap();
        let l1 = build_battle_lineup(&ms1, 4, 5, &pool, 7);
        assert_eq!((l1.min_tier, l1.max_tier), (1, 2));
        for id in l1.ally_tank_ids.iter().chain(l1.enemy_tank_ids.iter()) {
            let t = load_tank_spec(id).unwrap().tier;
            assert!(t == 1 || t == 2, "Tier 1 игрок не должен видеть Tier >= 3: {id} ({t})");
        }

        let pz4 = load_tank_spec("pz_4").unwrap();
        assert_eq!(pz4.tier, 3);
        let l3 = build_battle_lineup(&pz4, 4, 5, &pool, 19);
        assert_eq!((l3.min_tier, l3.max_tier), (2, 4));

        let t34_usa = load_tank_spec("t34_usa").unwrap();
        assert_eq!(t34_usa.tier, 7);
        let l7 = build_battle_lineup(&t34_usa, 4, 5, &pool, 31);
        assert_eq!((l7.min_tier, l7.max_tier), (6, 7));
        for id in l7.ally_tank_ids.iter().chain(l7.enemy_tank_ids.iter()) {
            let t = load_tank_spec(id).unwrap().tier;
            assert!(t == 6 || t == 7, "Tier 7 игрок должен видеть только Tier 6-7: {id} ({t})");
        }
    }
}
