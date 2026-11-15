//! Ориентированные бронеплиты танка и боевое состояние.

use std::f32::consts::{FRAC_PI_2, PI};

use bevy::prelude::*;

use super::spec::{ArmorZone, HullShape, TankSpec, TurretShape};
use crate::ballistics::{ArmorPlate, TargetInfo};
use crate::core::{GameState, Team};

/// Ссылка с сущности бронеплиты [`ArmorPlate`] на родительский танк и зону брони.
#[derive(Component, Debug, Clone, Copy)]
pub struct ArmorOwner {
    pub tank: Entity,
    pub zone: ArmorZone,
}

/// Боевое состояние танка и модификаторы от повреждений модулей / экипажа.
#[derive(Component, Debug, Clone)]
pub struct TankCombat {
    pub alive: bool,
    pub team: Team,
    pub tank_name: String,
    /// Счётчик пробитий в режиме `PrototypeTargets` (Этап 2).
    pub prototype_hits: u8,
    pub prototype_max_hits: u8,
    /// Множители ходовой, скорости башни/орудия, перезарядки и флаги клина.
    pub speed_mult: f32,
    pub traverse_mult: f32,
    pub elevation_mult: f32,
    pub reload_mult: f32,
    pub gun_jammed: bool,
    pub immobilized: bool,
}

impl TankCombat {
    pub fn new(team: Team, tank_name: String) -> Self {
        let prototype_max_hits = match team {
            Team::Enemy => 3,
            Team::Player => 5,
        };
        Self {
            alive: true,
            team,
            tank_name,
            prototype_hits: 0,
            prototype_max_hits,
            speed_mult: 1.0,
            traverse_mult: 1.0,
            elevation_mult: 1.0,
            reload_mult: 1.0,
            gun_jammed: false,
            immobilized: false,
        }
    }
}

/// Параметры отдельной бронеплиты танка в локальном кадре (корпуса или башни).
#[derive(Debug, Clone, Copy)]
pub struct TankPlateDesc {
    pub zone: ArmorZone,
    pub local_pos: Vec3,
    pub local_rot: Quat,
    pub half_extents: Vec2,
    pub thickness_mm: f32,
}

/// Строит локальные бронеплиты корпуса (относительно `TankRoot`).
pub fn hull_plate_specs(spec: &TankSpec) -> Vec<TankPlateDesc> {
    let p = &spec.profile;
    let a = &spec.armor;
    let hl = p.hull_length * 0.5;
    let hw = p.hull_width * 0.5;
    let inner_w = (hw - p.track_width * 0.45).max(0.50);
    let side_half_y = ((p.deck_height - 0.25) * 0.52).max(0.35);
    let side_y = 0.25 + side_half_y;

    let slope_rad = a.vld_slope_deg.clamp(0.0, 75.0).to_radians();
    // Локальная +Z плиты при повороте Y(PI) смотрит на −Z (вперёд), затем +X(slope) поднимает её вверх (+Y).
    let vld_rot = Quat::from_rotation_x(slope_rad) * Quat::from_rotation_y(PI);
    let nld_rot = Quat::from_rotation_x(-38f32.to_radians()) * Quat::from_rotation_y(PI);

    let mut plates = Vec::new();
    if p.hull_shape == HullShape::PikeNose {
        // «Щучий нос» ИС-3: левая и правая скулы ВЛД с разворотом по курсу и наклоном.
        let half_x = inner_w * 0.52;
        let pike_yaw = 24f32.to_radians();
        let right_rot =
            Quat::from_rotation_y(-pike_yaw) * Quat::from_rotation_x(slope_rad) * Quat::from_rotation_y(PI);
        let left_rot =
            Quat::from_rotation_y(pike_yaw) * Quat::from_rotation_x(slope_rad) * Quat::from_rotation_y(PI);
        plates.push(TankPlateDesc {
            zone: ArmorZone::HullFrontUpper,
            local_pos: Vec3::new(-half_x * 0.9, p.deck_height * 0.78, -hl * 0.84),
            local_rot: left_rot,
            half_extents: Vec2::new(half_x * 1.05, 0.46),
            thickness_mm: a.hull_front_upper_mm,
        });
        plates.push(TankPlateDesc {
            zone: ArmorZone::HullFrontUpper,
            local_pos: Vec3::new(half_x * 0.9, p.deck_height * 0.78, -hl * 0.84),
            local_rot: right_rot,
            half_extents: Vec2::new(half_x * 1.05, 0.46),
            thickness_mm: a.hull_front_upper_mm,
        });
    } else {
        plates.push(TankPlateDesc {
            zone: ArmorZone::HullFrontUpper,
            local_pos: Vec3::new(0.0, p.deck_height * 0.78, -hl * 0.90),
            local_rot: vld_rot,
            half_extents: Vec2::new(inner_w * 0.92, 0.46),
            thickness_mm: a.hull_front_upper_mm,
        });
    }

    plates.extend([
        TankPlateDesc {
            zone: ArmorZone::HullFrontLower,
            local_pos: Vec3::new(0.0, 0.50, -hl * 0.96),
            local_rot: nld_rot,
            half_extents: Vec2::new(inner_w * 0.92, 0.28),
            thickness_mm: a.hull_front_lower_mm,
        },
        TankPlateDesc {
            zone: ArmorZone::HullSideLeft,
            local_pos: Vec3::new(-inner_w, side_y, 0.05),
            local_rot: Quat::from_rotation_y(-FRAC_PI_2),
            half_extents: Vec2::new(hl * 0.90, side_half_y),
            thickness_mm: a.hull_side_mm,
        },
        TankPlateDesc {
            zone: ArmorZone::HullSideRight,
            local_pos: Vec3::new(inner_w, side_y, 0.05),
            local_rot: Quat::from_rotation_y(FRAC_PI_2),
            half_extents: Vec2::new(hl * 0.90, side_half_y),
            thickness_mm: a.hull_side_mm,
        },
        TankPlateDesc {
            zone: ArmorZone::HullRear,
            local_pos: Vec3::new(0.0, side_y + 0.05, hl * 0.94),
            local_rot: Quat::IDENTITY,
            half_extents: Vec2::new(inner_w * 0.92, side_half_y),
            thickness_mm: a.hull_rear_mm,
        },
        TankPlateDesc {
            zone: ArmorZone::HullRoof,
            local_pos: Vec3::new(0.0, p.deck_height, 0.10),
            local_rot: Quat::from_rotation_x(-FRAC_PI_2),
            half_extents: Vec2::new(inner_w * 0.90, hl * 0.80),
            thickness_mm: a.hull_roof_mm,
        },
    ]);
    plates
}

/// Строит локальные бронеплиты башни/рубки (относительно `TurretPivot`).
pub fn turret_plate_specs(spec: &TankSpec) -> Vec<TankPlateDesc> {
    let p = &spec.profile;
    let a = &spec.armor;
    let rx = p.turret_radius_x;
    let rz = p.turret_radius_z;
    let th = p.turret_height;

    let front_slope_deg: f32 = match p.turret_shape {
        TurretShape::FixedCasemate => 50.0,
        TurretShape::RoundedDome => 32.0,
        TurretShape::BoxHeavy => 6.0,
        _ => 16.0,
    };
    let front_rot = Quat::from_rotation_x(front_slope_deg.to_radians()) * Quat::from_rotation_y(PI);

    vec![
        TankPlateDesc {
            zone: ArmorZone::TurretFront,
            local_pos: Vec3::new(0.0, th * 0.5, -rz * 0.86),
            local_rot: front_rot,
            half_extents: Vec2::new(rx * 0.85, th * 0.52),
            thickness_mm: a.turret_front_mm,
        },
        TankPlateDesc {
            zone: ArmorZone::TurretSideLeft,
            local_pos: Vec3::new(-rx * 0.88, th * 0.5, 0.05),
            local_rot: Quat::from_rotation_y(-FRAC_PI_2),
            half_extents: Vec2::new(rz * 0.84, th * 0.50),
            thickness_mm: a.turret_side_mm,
        },
        TankPlateDesc {
            zone: ArmorZone::TurretSideRight,
            local_pos: Vec3::new(rx * 0.88, th * 0.5, 0.05),
            local_rot: Quat::from_rotation_y(FRAC_PI_2),
            half_extents: Vec2::new(rz * 0.84, th * 0.50),
            thickness_mm: a.turret_side_mm,
        },
        TankPlateDesc {
            zone: ArmorZone::TurretRear,
            local_pos: Vec3::new(0.0, th * 0.5, rz * 0.90),
            local_rot: Quat::IDENTITY,
            half_extents: Vec2::new(rx * 0.82, th * 0.50),
            thickness_mm: a.turret_rear_mm,
        },
        TankPlateDesc {
            zone: ArmorZone::TurretRoof,
            local_pos: Vec3::new(0.0, th, 0.05),
            local_rot: Quat::from_rotation_x(-FRAC_PI_2),
            half_extents: Vec2::new(rx * 0.80, rz * 0.82),
            thickness_mm: a.turret_roof_mm,
        },
    ]
}

/// Спавнит дочерние сущности бронеплит на корпусе (`root`) и башне (`turret_pivot`).
pub fn spawn_tank_armor_plates(
    commands: &mut Commands,
    root: Entity,
    turret_pivot: Entity,
    spec: &TankSpec,
) {
    let mut hull_children = Vec::new();
    for plate in hull_plate_specs(spec) {
        let e = commands
            .spawn((
                Name::new(format!("Armor {} {:?}", spec.name, plate.zone)),
                Transform::from_translation(plate.local_pos).with_rotation(plate.local_rot),
                ArmorPlate {
                    thickness_mm: plate.thickness_mm,
                    half_extents: plate.half_extents,
                },
                ArmorOwner {
                    tank: root,
                    zone: plate.zone,
                },
                TargetInfo {
                    name: format!("{} ({})", spec.name, plate.zone.ru_name()),
                },
                DespawnOnExit(GameState::InBattle),
            ))
            .id();
        hull_children.push(e);
    }
    commands.entity(root).add_children(&hull_children);

    let mut turret_children = Vec::new();
    for plate in turret_plate_specs(spec) {
        let e = commands
            .spawn((
                Name::new(format!("Armor {} {:?}", spec.name, plate.zone)),
                Transform::from_translation(plate.local_pos).with_rotation(plate.local_rot),
                ArmorPlate {
                    thickness_mm: plate.thickness_mm,
                    half_extents: plate.half_extents,
                },
                ArmorOwner {
                    tank: root,
                    zone: plate.zone,
                },
                TargetInfo {
                    name: format!("{} ({})", spec.name, plate.zone.ru_name()),
                },
                DespawnOnExit(GameState::InBattle),
            ))
            .id();
        turret_children.push(e);
    }
    commands.entity(turret_pivot).add_children(&turret_children);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tank::spec::{available_tanks, load_tank_spec};

    #[test]
    fn hull_and_turret_plate_normals_point_outward() {
        let spec = load_tank_spec("t34_85").unwrap();
        let hull = hull_plate_specs(&spec);
        let find_h = |z| hull.iter().find(|p| p.zone == z).unwrap();

        let vld_n = find_h(ArmorZone::HullFrontUpper).local_rot * Vec3::Z;
        // ВЛД: вперёд (-Z) и вверх (+Y) под углом ~60°.
        assert!(vld_n.z < -0.4 && vld_n.y > 0.8, "vld_n={vld_n:?}");
        assert!((vld_n.y - 60f32.to_radians().sin()).abs() < 1e-3);

        let nld_n = find_h(ArmorZone::HullFrontLower).local_rot * Vec3::Z;
        assert!(nld_n.z < -0.5 && nld_n.y < -0.5, "nld_n={nld_n:?}");

        let left_n = find_h(ArmorZone::HullSideLeft).local_rot * Vec3::Z;
        assert!(left_n.x < -0.99, "left_n={left_n:?}");

        let right_n = find_h(ArmorZone::HullSideRight).local_rot * Vec3::Z;
        assert!(right_n.x > 0.99, "right_n={right_n:?}");

        let rear_n = find_h(ArmorZone::HullRear).local_rot * Vec3::Z;
        assert!(rear_n.z > 0.99, "rear_n={rear_n:?}");

        let roof_n = find_h(ArmorZone::HullRoof).local_rot * Vec3::Z;
        assert!(roof_n.y > 0.99, "roof_n={roof_n:?}");

        let turret = turret_plate_specs(&spec);
        let tf = turret
            .iter()
            .find(|p| p.zone == ArmorZone::TurretFront)
            .unwrap();
        let tf_n = tf.local_rot * Vec3::Z;
        assert!(tf_n.z < -0.9 && tf_n.y > 0.1, "tf_n={tf_n:?}");
    }

    #[test]
    fn is3_pike_nose_has_two_angled_cheeks_and_su100_has_sloped_casemate() {
        let is3 = load_tank_spec("is_3").unwrap();
        let hull = hull_plate_specs(&is3);
        let vlds: Vec<_> = hull
            .iter()
            .filter(|p| p.zone == ArmorZone::HullFrontUpper)
            .collect();
        assert_eq!(vlds.len(), 2);
        let n_left = vlds[0].local_rot * Vec3::Z;
        let n_right = vlds[1].local_rot * Vec3::Z;
        assert!(n_left.x < -0.15 && n_left.z < -0.3 && n_left.y > 0.6, "n_left={n_left:?}");
        assert!(n_right.x > 0.15 && n_right.z < -0.3 && n_right.y > 0.6, "n_right={n_right:?}");

        let su = load_tank_spec("su_100").unwrap();
        let su_t = turret_plate_specs(&su);
        let tf = su_t.iter().find(|p| p.zone == ArmorZone::TurretFront).unwrap();
        let n = tf.local_rot * Vec3::Z;
        assert!(n.y > 0.7 && n.z < -0.5, "SU-100 front casemate n={n:?}");
    }

    #[test]
    fn all_fifteen_tanks_produce_valid_hull_and_turret_plates() {
        let tanks = available_tanks();
        assert_eq!(tanks.len(), 18);
        for spec in &tanks {
            let hull = hull_plate_specs(spec);
            let turret = turret_plate_specs(spec);
            assert!(hull.len() >= 6, "{} hull plates={}", spec.id, hull.len());
            assert_eq!(turret.len(), 5, "{} turret plates", spec.id);
            for plate in hull.iter().chain(turret.iter()) {
                assert!(plate.half_extents.x > 0.1 && plate.half_extents.y > 0.1, "{}: {:?}", spec.id, plate.zone);
                assert!(plate.thickness_mm > 0.0, "{}: {:?}", spec.id, plate.zone);
            }
        }
    }
}
