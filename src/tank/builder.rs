//! Процедурный конструктор воксельных сеток танка из [`ProceduralTankProfile`].

use bevy::math::{IVec3, UVec3, Vec3};

use super::spec::{HullShape, ProceduralTankProfile, TurretShape};
use crate::core::Team;
use crate::voxel::{Material, VoxelGrid};

/// Цветовая схема вокселей танка по команде.
#[derive(Debug, Clone, Copy)]
pub struct TankPalette {
    pub armor: Material,
    pub dark: Material,
    pub mark: Material,
}

impl TankPalette {
    pub fn for_team(team: Team) -> Self {
        match team {
            Team::Player => Self {
                armor: Material::TankOlive,
                dark: Material::TankDark,
                mark: Material::TankStar,
            },
            Team::Enemy => Self {
                armor: Material::EnemyArmor,
                dark: Material::EnemyDark,
                mark: Material::EnemyMark,
            },
        }
    }
}

/// Три части визуальной модели танка.
#[derive(Debug, Clone)]
pub struct TankVoxelParts {
    /// Корпус + гусеницы (в кадре `TankRoot`: origin — центр низа гусениц).
    pub hull: VoxelGrid,
    /// Башня (в кадре `TurretPivot`).
    pub turret: VoxelGrid,
    /// Ствол с маской (в кадре `GunPivot`: тянется вдоль −Z).
    pub barrel: VoxelGrid,
}

/// Строит сетку по предикату `f(p) -> Option<Material>` на боксе `[min_m, max_m]`.
pub fn build_from_fn(
    min_m: Vec3,
    max_m: Vec3,
    voxel_size: f32,
    mut f: impl FnMut(Vec3) -> Option<Material>,
) -> VoxelGrid {
    let size = (max_m - min_m).max(Vec3::splat(voxel_size));
    let dims = (size / voxel_size).ceil().as_uvec3().max(UVec3::ONE);
    let mut grid = VoxelGrid::new(dims, voxel_size, min_m);
    for y in 0..dims.y as i32 {
        for z in 0..dims.z as i32 {
            for x in 0..dims.x as i32 {
                let v = IVec3::new(x, y, z);
                let p = grid.voxel_center(v);
                if let Some(m) = f(p) {
                    grid.set(v, m);
                }
            }
        }
    }
    grid
}

/// Строит воксельные части танка по профилю (палитра игрока по умолчанию).
pub fn build_tank_parts(p: &ProceduralTankProfile) -> TankVoxelParts {
    build_tank_parts_for_team(p, Team::Player)
}

/// Строит воксельные части танка с учётом команды (окраска игрока/врага) и формы корпуса/башни.
pub fn build_tank_parts_for_team(p: &ProceduralTankProfile, team: Team) -> TankVoxelParts {
    let pal = TankPalette::for_team(team);
    let hl = p.hull_length * 0.5;
    let hw = p.hull_width * 0.5;
    let inner_w = hw - p.track_width;

    // 1. Корпус + гусеницы.
    let hull = build_from_fn(
        Vec3::new(-hw, 0.0, -hl),
        Vec3::new(hw, p.deck_height + 0.15, hl + 0.25),
        p.voxel_size,
        |pt| {
            let ax = pt.x.abs();
            // Гусеничные ленты по бокам (скруглённые спереди и сзади) + катки.
            if ax >= inner_w && ax <= hw && pt.y >= 0.0 && pt.y <= p.track_height {
                let z_lim = hl - 0.15;
                let in_end = if pt.z.abs() <= z_lim {
                    true
                } else {
                    let dz = pt.z.abs() - z_lim;
                    let dy = pt.y - p.track_height * 0.55;
                    dz * dz + dy * dy <= (p.track_height * 0.5) * (p.track_height * 0.5)
                };
                if in_end {
                    // 5 опорных катков внутри гусеницы.
                    for i in -2..=2 {
                        let wz = i as f32 * (p.hull_length * 0.16);
                        let wy = p.track_height * 0.42;
                        let r = (pt.z - wz).hypot(pt.y - wy);
                        if r <= p.track_height * 0.34 && ax < hw - p.voxel_size * 0.4 {
                            return Some(Material::TankWheel);
                        }
                    }
                    return Some(Material::TankTrack);
                }
            }

            // Надгусеничные полки.
            if ax >= inner_w * 0.95
                && ax <= hw
                && pt.y > p.track_height
                && pt.y <= p.track_height + p.voxel_size * 1.1
                && pt.z.abs() <= hl - 0.1
            {
                return Some(pal.dark);
            }

            // Бронекорпус между гусеницами.
            let clearance = 0.35;
            if pt.y < clearance || pt.y > p.deck_height {
                return None;
            }
            let slope_narrow = match p.hull_shape {
                HullShape::SlopedT34 | HullShape::Casemate => ((pt.y - 0.7).max(0.0)) * 0.32,
                HullShape::PikeNose => ((pt.y - 0.72).max(0.0)) * 0.30,
                HullShape::LightBox | HullShape::HeavyBox => 0.0,
            };
            if ax > inner_w - slope_narrow {
                return None;
            }

            let front_z = match p.hull_shape {
                HullShape::SlopedT34 => {
                    if pt.y >= 0.65 {
                        -hl + (pt.y - 0.65) * 1.15
                    } else {
                        -hl + (0.65 - pt.y) * 0.7
                    }
                }
                HullShape::PikeNose => {
                    let v = ax * 0.56;
                    if pt.y >= 0.72 {
                        -hl + v + (pt.y - 0.72) * 1.05
                    } else {
                        -hl + v + (0.72 - pt.y) * 0.70
                    }
                }
                HullShape::Casemate => {
                    if pt.y >= 0.65 {
                        -hl + (pt.y - 0.65) * 0.95
                    } else {
                        -hl + (0.65 - pt.y) * 0.65
                    }
                }
                HullShape::LightBox => {
                    if pt.y >= 0.65 {
                        -hl + (pt.y - 0.65) * 0.7
                    } else {
                        -hl + (0.65 - pt.y) * 0.4
                    }
                }
                HullShape::HeavyBox => {
                    if pt.y >= 0.75 {
                        -hl + 0.35 + (pt.y - 0.75) * 0.12
                    } else {
                        -hl + (0.75 - pt.y) * 0.85
                    }
                }
            };
            let rear_z = hl - (pt.y - clearance).max(0.0) * 0.25;
            if pt.z < front_z || pt.z > rear_z {
                return None;
            }
            // Внешние кормовые топливные бочки у ИС-3 и Т-34-85 на бортах.
            if matches!(p.hull_shape, HullShape::PikeNose)
                && ax > inner_w * 0.78
                && pt.z > hl * 0.25
                && pt.z < hl * 0.78
                && pt.y > p.deck_height - 0.28
            {
                return Some(pal.dark);
            }
            // Решётка МТО сверху сзади.
            if pt.y > p.deck_height - p.voxel_size * 1.2 && pt.z > 0.6 && ax < inner_w * 0.65 {
                return Some(pal.dark);
            }
            Some(pal.armor)
        },
    );

    // 2. Башня (или рубка ПТ-САУ).
    let rx = p.turret_radius_x;
    let rz = p.turret_radius_z;
    let th = p.turret_height;
    let turret = build_from_fn(
        Vec3::new(-rx - 0.15, 0.0, -rz - 0.2),
        Vec3::new(rx + 0.15, th + 0.3, rz + 0.45),
        p.voxel_size,
        |pt| {
            // Командирская башенка (для ПТ-САУ минимальный перископный лючок).
            let cup = Vec3::new(-rx * 0.35, th, rz * 0.25);
            let cup_h = if p.turret_shape == TurretShape::FixedCasemate {
                0.10
            } else if p.turret_shape == TurretShape::BoxHeavy {
                0.26
            } else {
                0.20
            };
            if pt.y >= th && pt.y <= th + cup_h {
                let d = ((pt.x - cup.x).powi(2) + (pt.z - cup.z).powi(2)).sqrt();
                let r_cup = if p.turret_shape == TurretShape::BoxHeavy {
                    0.32
                } else {
                    0.26
                };
                if d <= r_cup {
                    return Some(pal.dark);
                }
            }
            // Кормовой ящик (Rommelkiste) на Tiger I.
            if p.turret_shape == TurretShape::BoxHeavy
                && pt.y >= 0.12
                && pt.y <= th * 0.78
                && pt.x.abs() <= rx * 0.65
                && pt.z > rz * 0.90
                && pt.z <= rz + 0.32
            {
                return Some(pal.dark);
            }
            if pt.y < 0.0 || pt.y > th {
                return None;
            }
            let inside = match p.turret_shape {
                TurretShape::BoxHeavy => {
                    let k = 1.0 - (pt.y / th) * 0.06;
                    if pt.z < 0.0 {
                        let nx = pt.x / (rx * k);
                        let nz = pt.z / (rz * k);
                        nx * nx + nz * nz <= 1.0
                    } else {
                        pt.x.abs() <= rx * k && pt.z <= rz * k
                    }
                }
                TurretShape::FixedCasemate => {
                    let k = 1.0 - (pt.y / th) * 0.24;
                    let front_slope = -rz + (pt.y / th) * 0.72;
                    pt.x.abs() <= rx * k && pt.z >= front_slope && pt.z <= rz
                }
                TurretShape::RoundedDome => {
                    let t = (pt.y / th).clamp(0.0, 1.0);
                    let k = (1.0 - t.powf(1.8) * 0.44).max(0.45);
                    let nx = pt.x / (rx * k);
                    let nz = (pt.z - 0.06) / (rz * k);
                    nx * nx + nz * nz <= 1.0
                }
                TurretShape::CompactLight => {
                    let k = 1.0 - (pt.y / th) * 0.16;
                    let bustle = pt.z > rz * 0.35
                        && pt.z <= rz + 0.18
                        && pt.x.abs() <= rx * 0.68
                        && pt.y >= th * 0.20;
                    let nx = pt.x / (rx * k);
                    let nz = (pt.z - 0.02) / (rz * k);
                    bustle || (nx * nx + nz * nz <= 1.0)
                }
                TurretShape::HexCast => {
                    let k = 1.0 - (pt.y / th) * 0.20;
                    let nx = pt.x / (rx * k);
                    let nz = (pt.z - 0.05) / (rz * k);
                    nx * nx + nz * nz <= 1.0
                }
            };
            if !inside {
                return None;
            }
            // Опознавательный знак на бортах башни.
            if (pt.y - th * 0.5).abs() < 0.18
                && pt.z.abs() < 0.22
                && pt.x.abs() > rx * 0.72
            {
                return Some(pal.mark);
            }
            Some(pal.armor)
        },
    );

    // 3. Орудие + маска + дульный тормоз (у Tiger I и ИС-3).
    let br = p.barrel_radius;
    let bl = p.barrel_length;
    let has_muzzle_brake = matches!(
        p.turret_shape,
        TurretShape::BoxHeavy | TurretShape::RoundedDome
    );
    let mantlet_w = if p.turret_shape == TurretShape::FixedCasemate {
        0.42
    } else {
        0.30
    };
    let barrel = build_from_fn(
        Vec3::new(-0.46, -0.28, -bl - 0.1),
        Vec3::new(0.46, 0.28, 0.25),
        p.gun_voxel_size,
        |pt| {
            if pt.z >= -0.38 && pt.z <= 0.18 && pt.x.abs() <= mantlet_w && pt.y.abs() <= 0.22 {
                return Some(pal.dark);
            }
            if has_muzzle_brake
                && pt.z <= -bl + 0.28
                && pt.z >= -bl
                && pt.x.abs() <= br * 1.85
                && pt.y.abs() <= br * 1.35
            {
                return Some(pal.dark);
            }
            if pt.z <= 0.0 && pt.z >= -bl {
                let r = pt.x.hypot(pt.y);
                if r <= br {
                    return Some(Material::TankSteel);
                }
            }
            None
        },
    );

    TankVoxelParts {
        hull,
        turret,
        barrel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tank::spec::{T34_85_RON, TankSpec, available_tanks};
    use crate::voxel::mesher::mesh_grid;

    #[test]
    fn t34_85_ron_parses_and_builds_non_empty_meshes() {
        let spec = TankSpec::from_ron(T34_85_RON).expect("t34_85.ron должен парситься");
        assert_eq!(spec.name, "Т-34-85");
        let parts = build_tank_parts(&spec.profile);
        assert!(parts.hull.count_solid() > 500, "hull={}", parts.hull.count_solid());
        assert!(parts.turret.count_solid() > 80, "turret={}", parts.turret.count_solid());
        assert!(parts.barrel.count_solid() > 20, "barrel={}", parts.barrel.count_solid());
        assert!(!mesh_grid(&parts.hull).is_empty());
        assert!(!mesh_grid(&parts.turret).is_empty());
        assert!(!mesh_grid(&parts.barrel).is_empty());

        // Габариты корпуса соответствуют профилю (порядка 3×1.3×6 м).
        let sz = parts.hull.size_m();
        assert!((sz.x - spec.profile.hull_width).abs() < 0.5, "sz={sz:?}");
        assert!((sz.z - spec.profile.hull_length).abs() < 0.6, "sz={sz:?}");
    }

    #[test]
    fn all_five_tanks_build_distinct_non_empty_voxel_models() {
        let tanks = available_tanks();
        assert_eq!(tanks.len(), 5);
        let mut counts = Vec::new();
        for t in &tanks {
            let parts = build_tank_parts(&t.profile);
            let h = parts.hull.count_solid();
            let tr = parts.turret.count_solid();
            let b = parts.barrel.count_solid();
            assert!(h > 400 && tr > 60 && b > 15, "{}: {h}/{tr}/{b}", t.id);
            assert!(!mesh_grid(&parts.hull).is_empty());
            assert!(!mesh_grid(&parts.turret).is_empty());
            assert!(!mesh_grid(&parts.barrel).is_empty());
            counts.push((t.id.clone(), h, tr, b));
        }
        // Гарантируем различие моделей между лёгким M24 и тяжёлым Tiger I.
        let m24 = counts.iter().find(|c| c.0 == "m24_chaffee").unwrap();
        let tiger = counts.iter().find(|c| c.0 == "tiger_1").unwrap();
        assert!(tiger.1 > m24.1 && tiger.3 > m24.3);
    }
}
