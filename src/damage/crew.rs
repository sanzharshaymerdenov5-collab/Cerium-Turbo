//! Экипаж танка для режима War Thunder (`ModulesOnlyWt`, Этап 6):
//! 3D-рассадка (мехвод, наводчик, командир, заряжающий), выведение из строя,
//! пересадка на освободившееся место (7 с) и гибель при < 2 боеспособных танкистах.

use bevy::prelude::*;

use super::modules::ray_aabb;
use crate::tank::{TankCombat, TankSpec};

/// Позиция / роль члена экипажа.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CrewRole {
    Driver,
    Gunner,
    Commander,
    Loader,
}

impl CrewRole {
    pub fn ru_short(self) -> &'static str {
        match self {
            Self::Driver => "Мехвод",
            Self::Gunner => "Навод",
            Self::Commander => "Ком",
            Self::Loader => "Заряж",
        }
    }

    pub fn ru_name(self) -> &'static str {
        match self {
            Self::Driver => "Механик-водитель",
            Self::Gunner => "Наводчик",
            Self::Commander => "Командир",
            Self::Loader => "Заряжающий",
        }
    }
}

/// Состояние места члена экипажа.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CrewState {
    #[default]
    Alive,
    /// Пересел на место выбитого мехвода/наводчика (место свободно, но танкист жив!).
    Moved,
    Unconscious,
}

/// Один член экипажа в 3D-объёме корпуса или башни.
#[derive(Debug, Clone)]
pub struct CrewMember {
    pub role: CrewRole,
    pub hp: f32,
    pub max_hp: f32,
    pub state: CrewState,
    pub in_turret: bool,
    pub local_center: Vec3,
    pub half_extents: Vec3,
    /// Истинно, если место занимает заменивший коллега (даёт небольшой штраф к навыку).
    pub is_backup: bool,
}

impl CrewMember {
    pub fn apply_damage(&mut self, dmg: f32) -> (CrewState, CrewState) {
        let prev = self.state;
        if self.state != CrewState::Alive {
            return (prev, self.state);
        }
        self.hp = (self.hp - dmg.max(0.0)).max(0.0);
        if self.hp <= 0.0 {
            self.state = CrewState::Unconscious;
        }
        (prev, self.state)
    }
}

/// Экипаж на борту танка (4 танкиста).
#[derive(Component, Debug, Clone)]
pub struct TankCrew {
    pub members: Vec<CrewMember>,
    /// Таймер пересадки танкиста, с.
    pub replacement_timer: f32,
    /// Кого сейчас заменяют (приоритет: Наводчик -> Мехвод).
    pub replacing: Option<CrewRole>,
    /// Накопитель времени пожара для поражения экипажа в WT-режиме.
    pub fire_damage_acc: f32,
}

impl TankCrew {
    pub const REPLACEMENT_DURATION_S: f32 = 7.0;

    pub fn for_spec(spec: &TankSpec) -> Self {
        let p = &spec.profile;
        let hl = p.hull_length * 0.5;
        let hw = p.hull_width * 0.5;
        let inner_w = (hw - p.track_width * 0.5).max(0.50);
        let rx = p.turret_radius_x;
        let rz = p.turret_radius_z;
        let th = p.turret_height;

        let members = vec![
            CrewMember {
                role: CrewRole::Driver,
                hp: 45.0,
                max_hp: 45.0,
                state: CrewState::Alive,
                in_turret: false,
                local_center: Vec3::new(-inner_w * 0.34, 0.76, -hl * 0.50),
                half_extents: Vec3::new(0.24, 0.30, 0.34),
                is_backup: false,
            },
            CrewMember {
                role: CrewRole::Gunner,
                hp: 45.0,
                max_hp: 45.0,
                state: CrewState::Alive,
                in_turret: true,
                local_center: Vec3::new(-rx * 0.45, th * 0.46, -rz * 0.16),
                half_extents: Vec3::new(0.24, 0.29, 0.28),
                is_backup: false,
            },
            CrewMember {
                role: CrewRole::Commander,
                hp: 45.0,
                max_hp: 45.0,
                state: CrewState::Alive,
                in_turret: true,
                local_center: Vec3::new(-rx * 0.40, th * 0.54, rz * 0.32),
                half_extents: Vec3::new(0.24, 0.30, 0.28),
                is_backup: false,
            },
            CrewMember {
                role: CrewRole::Loader,
                hp: 45.0,
                max_hp: 45.0,
                state: CrewState::Alive,
                in_turret: true,
                local_center: Vec3::new(rx * 0.45, th * 0.48, 0.05),
                half_extents: Vec3::new(0.24, 0.29, 0.30),
                is_backup: false,
            },
        ];

        Self {
            members,
            replacement_timer: 0.0,
            replacing: None,
            fire_damage_acc: 0.0,
        }
    }

    pub fn state(&self, role: CrewRole) -> CrewState {
        self.members
            .iter()
            .find(|m| m.role == role)
            .map_or(CrewState::Unconscious, |m| m.state)
    }

    pub fn member(&self, role: CrewRole) -> Option<&CrewMember> {
        self.members.iter().find(|m| m.role == role)
    }

    pub fn member_mut(&mut self, role: CrewRole) -> Option<&mut CrewMember> {
        self.members.iter_mut().find(|m| m.role == role)
    }

    /// Сколько танкистов сейчас в сознании.
    pub fn alive_count(&self) -> usize {
        self.members
            .iter()
            .filter(|m| m.state == CrewState::Alive)
            .count()
    }

    pub fn total_count(&self) -> usize {
        self.members.len()
    }

    fn has_spare_donor(&self) -> bool {
        self.state(CrewRole::Commander) == CrewState::Alive
            || self.state(CrewRole::Loader) == CrewState::Alive
    }

    /// Обновляет таймер пересадки при потере наводчика или мехвода.
    /// Возвращает роль, на которую только что пересел танкист.
    pub fn tick_replacement(&mut self, dt: f32) -> Option<CrewRole> {
        if self.alive_count() < 2 {
            self.replacing = None;
            self.replacement_timer = 0.0;
            return None;
        }

        // Если текущий кандидат на замену больше не требует пересадки или нет донора -> сбрасываем.
        if let Some(target) = self.replacing
            && (self.state(target) == CrewState::Alive || !self.has_spare_donor())
        {
            self.replacing = None;
            self.replacement_timer = 0.0;
        }

        // Приоритет замены: Наводчик -> Механик-водитель.
        if self.replacing.is_none() && self.has_spare_donor() {
            if self.state(CrewRole::Gunner) != CrewState::Alive {
                self.replacing = Some(CrewRole::Gunner);
                self.replacement_timer = Self::REPLACEMENT_DURATION_S;
            } else if self.state(CrewRole::Driver) != CrewState::Alive {
                self.replacing = Some(CrewRole::Driver);
                self.replacement_timer = Self::REPLACEMENT_DURATION_S;
            }
        }

        if let Some(target) = self.replacing {
            self.replacement_timer -= dt;
            if self.replacement_timer <= 0.0 {
                let donor = if self.state(CrewRole::Commander) == CrewState::Alive {
                    Some(CrewRole::Commander)
                } else if self.state(CrewRole::Loader) == CrewState::Alive {
                    Some(CrewRole::Loader)
                } else {
                    None
                };
                self.replacing = None;
                self.replacement_timer = 0.0;

                if let Some(d_role) = donor {
                    if let Some(d) = self.member_mut(d_role) {
                        d.state = CrewState::Moved;
                        d.hp = 0.0;
                    }
                    if let Some(t) = self.member_mut(target) {
                        t.state = CrewState::Alive;
                        t.hp = t.max_hp;
                        t.is_backup = true;
                    }
                    return Some(target);
                }
            }
        }
        None
    }

    /// Накладывает штрафы экипажа ПОВЕРХ штрафов модулей.
    pub fn apply_penalties_to_combat(&self, combat: &mut TankCombat) {
        if !combat.alive || self.alive_count() < 2 {
            combat.immobilized = true;
            combat.gun_jammed = true;
            combat.speed_mult = 0.0;
            combat.traverse_mult = 0.0;
            combat.elevation_mult = 0.0;
            return;
        }

        let Some(driver) = self.member(CrewRole::Driver) else {
            return;
        };
        if driver.state != CrewState::Alive {
            combat.immobilized = true;
            combat.speed_mult = 0.0;
        } else if driver.is_backup {
            combat.speed_mult *= 0.80;
        }

        let Some(gunner) = self.member(CrewRole::Gunner) else {
            return;
        };
        if gunner.state != CrewState::Alive {
            combat.gun_jammed = true;
            combat.traverse_mult = 0.0;
            combat.elevation_mult = 0.0;
        } else if gunner.is_backup {
            combat.traverse_mult *= 0.72;
            combat.elevation_mult *= 0.72;
        }

        if self.state(CrewRole::Loader) != CrewState::Alive {
            combat.reload_mult *= 1.65;
        }
    }

    /// Трассирует мировой луч `(origin, dir, max_dist)` по 3D-объёмам экипажа.
    pub fn raycast_crew(
        &self,
        hull_gt: &GlobalTransform,
        turret_gt: Option<&GlobalTransform>,
        origin: Vec3,
        dir: Vec3,
        max_dist: f32,
    ) -> Vec<(usize, f32)> {
        let inv_hull = hull_gt.affine().inverse();
        let inv_turret = turret_gt.map(|gt| gt.affine().inverse());
        let mut hits = Vec::new();

        for (idx, member) in self.members.iter().enumerate() {
            if member.state != CrewState::Alive {
                continue;
            }
            let affine = if member.in_turret {
                inv_turret.unwrap_or(inv_hull)
            } else {
                inv_hull
            };
            let lo = affine.transform_point3(origin);
            let ld = affine.transform_vector3(dir).normalize_or_zero();
            if let Some(t) = ray_aabb(lo, ld, max_dist, member.local_center, member.half_extents) {
                hits.push((idx, t));
            }
        }
        hits.sort_by(|a, b| a.1.total_cmp(&b.1));
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Team;
    use crate::damage::TankModules;
    use crate::tank::load_tank_spec;

    #[test]
    fn crew_replacement_transfers_commander_to_gunner_after_timer() {
        let spec = load_tank_spec("t34_85").unwrap();
        let mods = TankModules::for_spec(&spec);
        let mut crew = TankCrew::for_spec(&spec);
        let mut combat = TankCombat::new(Team::Player, spec.name.clone());

        assert_eq!(crew.alive_count(), 4);

        // Выводим наводчика из строя -> пока идёт замена, наводить и стрелять нельзя.
        crew.member_mut(CrewRole::Gunner).unwrap().apply_damage(100.0);
        assert_eq!(crew.alive_count(), 3);
        crew.tick_replacement(0.5);
        assert_eq!(crew.replacing, Some(CrewRole::Gunner));
        mods.apply_penalties_to_combat(&mut combat);
        crew.apply_penalties_to_combat(&mut combat);
        assert!(combat.gun_jammed);
        assert_eq!(combat.traverse_mult, 0.0);
        assert_eq!(combat.elevation_mult, 0.0);

        // Проходит ещё 7 секунд -> командир пересаживается в наводчики!
        let moved = crew.tick_replacement(7.0);
        assert_eq!(moved, Some(CrewRole::Gunner));
        assert_eq!(crew.alive_count(), 3);
        assert_eq!(crew.state(CrewRole::Gunner), CrewState::Alive);
        assert_eq!(crew.state(CrewRole::Commander), CrewState::Moved);

        mods.apply_penalties_to_combat(&mut combat);
        crew.apply_penalties_to_combat(&mut combat);
        assert!(!combat.gun_jammed);
        assert!(combat.traverse_mult > 0.6 && combat.traverse_mult < 0.8);
    }

    #[test]
    fn tank_is_incapacitated_when_fewer_than_two_crew_alive() {
        let spec = load_tank_spec("tiger_1").unwrap();
        let mut crew = TankCrew::for_spec(&spec);
        let mut combat = TankCombat::new(Team::Enemy, spec.name.clone());

        for role in [CrewRole::Driver, CrewRole::Gunner, CrewRole::Commander] {
            crew.member_mut(role).unwrap().apply_damage(100.0);
        }
        assert_eq!(crew.alive_count(), 1);
        crew.apply_penalties_to_combat(&mut combat);
        assert!(combat.immobilized);
        assert!(combat.gun_jammed);
    }
}
