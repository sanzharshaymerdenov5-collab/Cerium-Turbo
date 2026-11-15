//! Внутренние и ходовые модули танка: 3D-объёмы, состояние (`Intact/Damaged/Destroyed`), штрафы и трассировка.

use bevy::prelude::*;

use crate::tank::{ArmorZone, TankCombat, TankSpec};

/// Тип модуля танка.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleKind {
    LeftTrack,
    RightTrack,
    Engine,
    TurretRing,
    GunBreech,
    AmmoRack,
    FuelTank,
}

impl ModuleKind {
    pub fn ru_short(self) -> &'static str {
        match self {
            Self::LeftTrack => "Л.Гус",
            Self::RightTrack => "П.Гус",
            Self::Engine => "Двиг",
            Self::TurretRing => "Погон",
            Self::GunBreech => "Орудие",
            Self::AmmoRack => "БК",
            Self::FuelTank => "Баки",
        }
    }

    pub fn ru_name(self) -> &'static str {
        match self {
            Self::LeftTrack => "Левая гусеница",
            Self::RightTrack => "Правая гусеница",
            Self::Engine => "Двигатель",
            Self::TurretRing => "Погон башни",
            Self::GunBreech => "Казённик орудия",
            Self::AmmoRack => "Боеукладка",
            Self::FuelTank => "Топливный бак",
        }
    }
}

/// Состояние модуля.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModuleState {
    #[default]
    Intact,
    Damaged,
    Destroyed,
}

/// Внутренний модуль в локальном кадре корпуса или башни.
#[derive(Debug, Clone)]
pub struct InternalModule {
    pub kind: ModuleKind,
    pub hp: f32,
    pub max_hp: f32,
    pub state: ModuleState,
    pub in_turret: bool,
    pub local_center: Vec3,
    pub half_extents: Vec3,
    /// Таймер авторемонта из `Destroyed` обратно в `Damaged`, с.
    pub repair_timer: f32,
}

impl InternalModule {
    pub fn apply_damage(&mut self, dmg: f32) -> (ModuleState, ModuleState) {
        let prev = self.state;
        self.hp = (self.hp - dmg.max(0.0)).max(0.0);
        if self.hp <= 0.0 {
            self.state = ModuleState::Destroyed;
            if self.repair_timer <= 0.0 {
                self.repair_timer = match self.kind {
                    ModuleKind::LeftTrack | ModuleKind::RightTrack => 5.5,
                    ModuleKind::Engine | ModuleKind::TurretRing | ModuleKind::GunBreech => 6.5,
                    ModuleKind::AmmoRack | ModuleKind::FuelTank => 0.0,
                };
            }
        } else if self.hp < self.max_hp * 0.55 && self.state == ModuleState::Intact {
            self.state = ModuleState::Damaged;
        }
        (prev, self.state)
    }

    pub fn repair_full(&mut self) {
        self.hp = self.max_hp;
        self.state = ModuleState::Intact;
        self.repair_timer = 0.0;
    }
}

/// Набор модулей на борту танка.
#[derive(Component, Debug, Clone)]
pub struct TankModules {
    pub modules: Vec<InternalModule>,
    /// Остаток горения топливных баков, с (0 = не горит).
    pub fire_timer: f32,
    /// Заряды ручного ремкомплекта (`[5]`).
    pub repair_charges: u8,
    /// Заряды ручного огнетушителя (`[6]`).
    pub extinguisher_charges: u8,
}

impl TankModules {
    pub fn for_spec(spec: &TankSpec) -> Self {
        let p = &spec.profile;
        let hl = p.hull_length * 0.5;
        let hw = p.hull_width * 0.5;
        let inner_w = (hw - p.track_width * 0.5).max(0.50);
        let rx = p.turret_radius_x;
        let rz = p.turret_radius_z;
        let th = p.turret_height;

        let scale = (spec.max_hp / 850.0).clamp(0.7, 1.8);
        let modules = vec![
            InternalModule {
                kind: ModuleKind::LeftTrack,
                hp: 140.0 * scale,
                max_hp: 140.0 * scale,
                state: ModuleState::Intact,
                in_turret: false,
                local_center: Vec3::new(-(hw - p.track_width * 0.5), p.track_height * 0.5, 0.0),
                half_extents: Vec3::new(p.track_width * 0.55, p.track_height * 0.5, hl * 0.92),
                repair_timer: 0.0,
            },
            InternalModule {
                kind: ModuleKind::RightTrack,
                hp: 140.0 * scale,
                max_hp: 140.0 * scale,
                state: ModuleState::Intact,
                in_turret: false,
                local_center: Vec3::new(hw - p.track_width * 0.5, p.track_height * 0.5, 0.0),
                half_extents: Vec3::new(p.track_width * 0.55, p.track_height * 0.5, hl * 0.92),
                repair_timer: 0.0,
            },
            InternalModule {
                kind: ModuleKind::Engine,
                hp: 160.0 * scale,
                max_hp: 160.0 * scale,
                state: ModuleState::Intact,
                in_turret: false,
                local_center: Vec3::new(0.0, 0.72, hl * 0.56),
                half_extents: Vec3::new(inner_w * 0.55, 0.36, hl * 0.30),
                repair_timer: 0.0,
            },
            InternalModule {
                kind: ModuleKind::FuelTank,
                hp: 130.0 * scale,
                max_hp: 130.0 * scale,
                state: ModuleState::Intact,
                in_turret: false,
                local_center: Vec3::new(inner_w * 0.42, 0.70, hl * 0.12),
                half_extents: Vec3::new(0.32, 0.32, hl * 0.32),
                repair_timer: 0.0,
            },
            InternalModule {
                kind: ModuleKind::AmmoRack,
                hp: 150.0 * scale,
                max_hp: 150.0 * scale,
                state: ModuleState::Intact,
                in_turret: false,
                local_center: Vec3::new(-inner_w * 0.28, 0.68, -hl * 0.15),
                half_extents: Vec3::new(0.45, 0.32, 0.72),
                repair_timer: 0.0,
            },
            InternalModule {
                kind: ModuleKind::TurretRing,
                hp: 135.0 * scale,
                max_hp: 135.0 * scale,
                state: ModuleState::Intact,
                in_turret: true,
                local_center: Vec3::new(0.0, 0.14, 0.0),
                half_extents: Vec3::new(rx * 0.72, 0.18, rz * 0.72),
                repair_timer: 0.0,
            },
            InternalModule {
                kind: ModuleKind::GunBreech,
                hp: 130.0 * scale,
                max_hp: 130.0 * scale,
                state: ModuleState::Intact,
                in_turret: true,
                local_center: Vec3::new(0.0, th * 0.5, -rz * 0.35),
                half_extents: Vec3::new(0.38, 0.28, 0.55),
                repair_timer: 0.0,
            },
        ];
        Self {
            modules,
            fire_timer: 0.0,
            repair_charges: 2,
            extinguisher_charges: 2,
        }
    }

    pub fn state(&self, kind: ModuleKind) -> ModuleState {
        self.modules
            .iter()
            .find(|m| m.kind == kind)
            .map_or(ModuleState::Intact, |m| m.state)
    }

    pub fn get_mut(&mut self, kind: ModuleKind) -> Option<&mut InternalModule> {
        self.modules.iter_mut().find(|m| m.kind == kind)
    }

    pub fn repair_all(&mut self) {
        for m in &mut self.modules {
            m.repair_full();
        }
    }

    /// Пересчитывает штрафы к подвижности, скорости башни и перезарядке.
    pub fn apply_penalties_to_combat(&self, combat: &mut TankCombat) {
        if !combat.alive {
            combat.immobilized = true;
            combat.gun_jammed = true;
            combat.speed_mult = 0.0;
            combat.traverse_mult = 0.0;
            combat.elevation_mult = 0.0;
            return;
        }
        let lt = self.state(ModuleKind::LeftTrack);
        let rt = self.state(ModuleKind::RightTrack);
        let eng = self.state(ModuleKind::Engine);
        let ring = self.state(ModuleKind::TurretRing);
        let gun = self.state(ModuleKind::GunBreech);
        let ammo = self.state(ModuleKind::AmmoRack);

        let tracks_down = lt == ModuleState::Destroyed || rt == ModuleState::Destroyed;
        let engine_down = eng == ModuleState::Destroyed;
        combat.immobilized = tracks_down || engine_down;

        combat.speed_mult = if combat.immobilized {
            0.0
        } else if eng == ModuleState::Damaged {
            0.55
        } else {
            1.0
        };

        combat.traverse_mult = match ring {
            ModuleState::Destroyed => 0.0,
            ModuleState::Damaged => 0.5,
            ModuleState::Intact => 1.0,
        };
        combat.elevation_mult = 1.0;

        combat.gun_jammed = gun == ModuleState::Destroyed;

        let mut reload_mult = 1.0;
        if gun == ModuleState::Damaged {
            reload_mult *= 1.5;
        }
        if ammo == ModuleState::Damaged {
            reload_mult *= 2.0;
        }
        combat.reload_mult = reload_mult;
    }

    /// Проверяет пересечение мирового луча `(origin, dir, max_dist)` со всеми модулями танка.
    /// Возвращает отсортированный список `(индекс_модуля, dist)`.
    pub fn raycast_modules(
        &self,
        hull_gt: &GlobalTransform,
        turret_gt: Option<&GlobalTransform>,
        origin: Vec3,
        dir: Vec3,
        max_dist: f32,
        hit_zone: Option<ArmorZone>,
        hit_height_local: f32,
    ) -> Vec<(usize, f32)> {
        let inv_hull = hull_gt.affine().inverse();
        let inv_turret = turret_gt.map(|gt| gt.affine().inverse());
        let mut hits = Vec::new();

        for (idx, m) in self.modules.iter().enumerate() {
            let affine = if m.in_turret {
                inv_turret.unwrap_or(inv_hull)
            } else {
                inv_hull
            };
            let lo = affine.transform_point3(origin);
            let ld = affine.transform_vector3(dir).normalize_or_zero();
            if let Some(t) = ray_aabb(lo, ld, max_dist, m.local_center, m.half_extents) {
                hits.push((idx, t));
                continue;
            }
            // Попадание по бортовой нижней проекции задевает соответствующую гусеницу.
            if hit_height_local < 0.85 {
                if (m.kind == ModuleKind::LeftTrack && hit_zone == Some(ArmorZone::HullSideLeft))
                    || (m.kind == ModuleKind::RightTrack
                        && hit_zone == Some(ArmorZone::HullSideRight))
                {
                    hits.push((idx, 0.2));
                }
            }
        }
        hits.sort_by(|a, b| a.1.total_cmp(&b.1));
        hits
    }
}

/// Пересечение луча с локальным AABB `[center - half, center + half]`.
pub fn ray_aabb(origin: Vec3, dir: Vec3, max_dist: f32, center: Vec3, half: Vec3) -> Option<f32> {
    let min = center - half;
    let max = center + half;
    let mut tmin = 0.0f32;
    let mut tmax = max_dist;
    for a in 0..3 {
        let d = dir[a];
        if d.abs() < 1e-6 {
            if origin[a] < min[a] || origin[a] > max[a] {
                return None;
            }
        } else {
            let inv = 1.0 / d;
            let mut t1 = (min[a] - origin[a]) * inv;
            let mut t2 = (max[a] - origin[a]) * inv;
            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }
            tmin = tmin.max(t1);
            tmax = tmax.min(t2);
            if tmin > tmax {
                return None;
            }
        }
    }
    Some(tmin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Team;
    use crate::tank::load_tank_spec;

    #[test]
    fn module_damage_applies_penalties_and_auto_repairs() {
        let spec = load_tank_spec("t34_85").unwrap();
        let mut mods = TankModules::for_spec(&spec);
        let mut combat = TankCombat::new(Team::Player, "Т-34-85".into());

        // Критический урон двигателю -> Damaged (-45% скорости).
        mods.get_mut(ModuleKind::Engine).unwrap().apply_damage(100.0);
        mods.apply_penalties_to_combat(&mut combat);
        assert!(!combat.immobilized);
        assert!((combat.speed_mult - 0.55).abs() < 1e-3);

        // Выведение гусеницы из строя -> обездвижен.
        mods.get_mut(ModuleKind::LeftTrack).unwrap().apply_damage(500.0);
        mods.apply_penalties_to_combat(&mut combat);
        assert!(combat.immobilized);
        assert_eq!(combat.speed_mult, 0.0);

        // Повреждённая боеукладка -> перезарядка x2.
        mods.get_mut(ModuleKind::AmmoRack).unwrap().apply_damage(90.0);
        mods.apply_penalties_to_combat(&mut combat);
        assert_eq!(mods.state(ModuleKind::AmmoRack), ModuleState::Damaged);
        assert!((combat.reload_mult - 2.0).abs() < 1e-3);

        // Ремкомплект снимает все штрафы.
        mods.repair_all();
        mods.apply_penalties_to_combat(&mut combat);
        assert!(!combat.immobilized);
        assert_eq!(combat.speed_mult, 1.0);
        assert_eq!(combat.reload_mult, 1.0);
    }

    #[test]
    fn ray_aabb_intersects_engine_from_rear() {
        let spec = load_tank_spec("t34_85").unwrap();
        let mods = TankModules::for_spec(&spec);
        let hull_gt = GlobalTransform::IDENTITY;
        let hits = mods.raycast_modules(
            &hull_gt,
            None,
            Vec3::new(0.0, 0.72, 3.0),
            Vec3::NEG_Z,
            4.5,
            Some(ArmorZone::HullRear),
            0.72,
        );
        assert!(
            hits.iter().any(|&(i, _)| mods.modules[i].kind == ModuleKind::Engine),
            "выстрел в корму должен задевать двигатель"
        );
    }
}
