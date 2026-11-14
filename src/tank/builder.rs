//! Процедурный конструктор воксельных сеток танка из [`ProceduralTankProfile`].

use bevy::math::{IVec3, UVec3, Vec3};

use super::spec::ProceduralTankProfile;
use crate::voxel::{Material, VoxelGrid};

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

/// Строит воксельные части танка по профилю (Т-34-85 и будущие танки Этапа 4).
pub fn build_tank_parts(p: &ProceduralTankProfile) -> TankVoxelParts {
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
                return Some(Material::TankDark);
            }

            // Бронекорпус между гусеницами (клиренс ~0.35 м, наклонные ВЛД/НЛД и борта).
            let clearance = 0.35;
            if pt.y < clearance || pt.y > p.deck_height {
                return None;
            }
            // Скошенные верхние борта (Т-34).
            let slope_narrow = ((pt.y - 0.7).max(0.0)) * 0.32;
            if ax > inner_w - slope_narrow {
                return None;
            }
            // ВЛД (верхняя лобовая деталь под углом ~60°) и НЛД.
            let front_z = if pt.y >= 0.65 {
                -hl + (pt.y - 0.65) * 1.15
            } else {
                -hl + (0.65 - pt.y) * 0.7
            };
            let rear_z = hl - (pt.y - clearance).max(0.0) * 0.25;
            if pt.z < front_z || pt.z > rear_z {
                return None;
            }
            // Решётка МТО сверху сзади.
            if pt.y > p.deck_height - p.voxel_size * 1.2 && pt.z > 0.6 && ax < inner_w * 0.65 {
                return Some(Material::TankDark);
            }
            Some(Material::TankOlive)
        },
    );

    // 2. Башня (вытянутый шестигранник/эллипсоид + командирская башенка + звезда).
    let rx = p.turret_radius_x;
    let rz = p.turret_radius_z;
    let th = p.turret_height;
    let turret = build_from_fn(
        Vec3::new(-rx - 0.1, 0.0, -rz - 0.2),
        Vec3::new(rx + 0.1, th + 0.3, rz + 0.25),
        p.voxel_size,
        |pt| {
            // Командирская башенка слева сзади.
            let cup = Vec3::new(-rx * 0.35, th, rz * 0.25);
            if pt.y >= th && pt.y <= th + 0.22 {
                let d = ((pt.x - cup.x).powi(2) + (pt.z - cup.z).powi(2)).sqrt();
                if d <= 0.28 {
                    return Some(Material::TankDark);
                }
            }
            if pt.y < 0.0 || pt.y > th {
                return None;
            }
            // Сужение башни кверху.
            let k = 1.0 - (pt.y / th) * 0.18;
            let nx = pt.x / (rx * k);
            let nz = (pt.z - 0.05) / (rz * k);
            if nx * nx + nz * nz > 1.0 {
                return None;
            }
            // Красная звезда на бортах башни.
            if (pt.y - th * 0.5).abs() < 0.18
                && pt.z.abs() < 0.22
                && pt.x.abs() > rx * k * 0.78
            {
                return Some(Material::TankStar);
            }
            Some(Material::TankOlive)
        },
    );

    // 3. Орудие + маска (вдоль −Z от цапф).
    let br = p.barrel_radius;
    let bl = p.barrel_length;
    let barrel = build_from_fn(
        Vec3::new(-0.35, -0.25, -bl),
        Vec3::new(0.35, 0.25, 0.2),
        p.gun_voxel_size,
        |pt| {
            // Маска орудия у основания.
            if pt.z >= -0.35 && pt.z <= 0.15 && pt.x.abs() <= 0.30 && pt.y.abs() <= 0.20 {
                return Some(Material::TankDark);
            }
            // Ствол (цилиндр вдоль −Z).
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
    use crate::tank::spec::{T34_85_RON, TankSpec};
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
}
