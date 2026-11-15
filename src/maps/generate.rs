//! Генератор воксельной сетки карты: неразрушаемый рельеф + разрушаемые постройки.

use bevy::math::{IVec3, UVec3, Vec3};

use super::spec::{MapBiome, MapSpec, StructureSpec};
use crate::voxel::{Material, VoxelGrid};

/// Высота рельефа (в метрах) в мировой точке `(x, z)`.
pub fn terrain_height_m(spec: &MapSpec, x: f32, z: f32) -> f32 {
    // Плавный коэффициент выравнивания в плоских зонах и ледяных озёрах.
    let mut flat_blend = 0.0f32;
    for fz in spec.flat_zones.iter().chain(spec.ice_zones.iter()) {
        let dx = if x < fz.min_xz.0 {
            fz.min_xz.0 - x
        } else if x > fz.max_xz.0 {
            x - fz.max_xz.0
        } else {
            0.0
        };
        let dz = if z < fz.min_xz.1 {
            fz.min_xz.1 - z
        } else if z > fz.max_xz.1 {
            z - fz.max_xz.1
        } else {
            0.0
        };
        let d = dx.hypot(dz);
        let b = (1.0 - (d / 8.0).clamp(0.0, 1.0)).powi(2);
        flat_blend = flat_blend.max(b);
    }

    let mut h = spec.base_height_m;
    // Мягкая волнистость поля.
    let undulation = (x * 0.045).sin() * (z * 0.035).cos() * 0.6;
    h += undulation * (1.0 - flat_blend);

    for hill in &spec.hills {
        let d2 = (x - hill.center_xz.0).powi(2) + (z - hill.center_xz.1).powi(2);
        let r2 = hill.radius_m * hill.radius_m;
        if d2 < r2 {
            let t = 1.0 - d2 / r2;
            h += hill.height_m * t * t * (1.0 - flat_blend);
        }
    }

    for ditch in &spec.ditches {
        if x >= ditch.min_xz.0
            && x <= ditch.max_xz.0
            && z >= ditch.min_xz.1
            && z <= ditch.max_xz.1
        {
            h -= ditch.depth_m;
        }
    }

    h.max(spec.voxel_size)
}

/// Строит воксельную сетку карты.
pub fn generate_map_grid(spec: &MapSpec) -> VoxelGrid {
    let dims = UVec3::new(spec.dims.0, spec.dims.1, spec.dims.2);
    let s = spec.voxel_size;
    let origin = Vec3::new(
        -(dims.x as f32) * s * 0.5,
        0.0,
        -(dims.z as f32) * s * 0.5,
    );
    let mut grid = VoxelGrid::new(dims, s, origin);

    // 1. Неразрушаемый рельеф (камень / грунт / трава / песок / снег / лёд).
    for z in 0..dims.z as i32 {
        for x in 0..dims.x as i32 {
            let wx = origin.x + (x as f32 + 0.5) * s;
            let wz = origin.z + (z as f32 + 0.5) * s;
            let h_m = terrain_height_m(spec, wx, wz);
            let top_y = ((h_m / s).round() as i32 - 1).clamp(0, dims.y as i32 - 2);
            let is_lane = spec.id == super::spec::PROVING_GROUNDS_ID
                && wx.abs() < 4.5
                && wz > -140.0
                && wz < 130.0;
            let in_ice = spec
                .ice_zones
                .iter()
                .any(|iz| wx >= iz.min_xz.0 && wx <= iz.max_xz.0 && wz >= iz.min_xz.1 && wz <= iz.max_xz.1);
            let high_rock = top_y as f32 * s > spec.base_height_m + 3.5;

            let surface_mat = if in_ice {
                Material::Ice
            } else {
                match spec.biome {
                    MapBiome::Temperate => {
                        let dirt_road = spec.id != super::spec::PROVING_GROUNDS_ID
                            && (wx - (wz * 0.032).sin() * 14.0).abs() < 3.0;
                        if is_lane {
                            Material::Sand
                        } else if high_rock {
                            Material::Rock
                        } else if dirt_road {
                            Material::Soil
                        } else {
                            Material::Grass
                        }
                    }
                    MapBiome::City => {
                        // Центральные проезжие улицы из каменных плит (неразрушаемые).
                        if wx.abs() < 12.0 || wz.abs() < 12.0 {
                            Material::Rock
                        } else {
                            Material::Soil
                        }
                    }
                    MapBiome::Desert => {
                        if high_rock {
                            Material::Rock
                        } else {
                            Material::Sand
                        }
                    }
                    MapBiome::Winter => {
                        if high_rock {
                            Material::Rock
                        } else {
                            Material::Snow
                        }
                    }
                    MapBiome::Industrial => {
                        // Насыпи и полосы путей — камень, земля между путями — грунт.
                        if (wx.abs() - 16.0).abs() < 3.5 || wx.abs() < 4.0 {
                            Material::Rock
                        } else {
                            Material::Soil
                        }
                    }
                    MapBiome::Highland => {
                        let elev = top_y as f32 * s - spec.base_height_m;
                        if elev > 8.5 {
                            Material::Snow
                        } else if elev > 2.2 || (wx.abs() > 62.0 && wx.abs() < 74.0) {
                            Material::Rock
                        } else {
                            Material::Grass
                        }
                    }
                }
            };

            for y in 0..=top_y {
                let mat = if y == 0 {
                    Material::Rock
                } else if y == top_y {
                    surface_mat
                } else {
                    Material::Soil
                };
                grid.set(IVec3::new(x, y, z), mat);
            }
        }
    }

    // 2. Разрушаемые постройки и укрытия.
    for st in &spec.structures {
        match *st {
            StructureSpec::Wall {
                min_xz,
                max_xz,
                height_m,
                kind,
            } => {
                let mat = kind.to_material();
                let vmin = grid.world_to_voxel(Vec3::new(min_xz.0, 0.0, min_xz.1));
                let vmax = grid.world_to_voxel(Vec3::new(max_xz.0, 0.0, max_xz.1));
                let hy = (height_m / s).ceil() as i32;
                for z in vmin.z.min(vmax.z)..=vmin.z.max(vmax.z) {
                    for x in vmin.x.min(vmax.x)..=vmin.x.max(vmax.x) {
                        let wx = origin.x + (x as f32 + 0.5) * s;
                        let wz = origin.z + (z as f32 + 0.5) * s;
                        let base_y = (terrain_height_m(spec, wx, wz) / s).round() as i32;
                        for y in base_y..(base_y + hy).min(dims.y as i32) {
                            grid.set(IVec3::new(x, y, z), mat);
                        }
                    }
                }
            }
            StructureSpec::House {
                center_xz,
                size_xz,
                height_m,
                kind,
            } => {
                let wall_mat = kind.to_material();
                let min_x = center_xz.0 - size_xz.0 * 0.5;
                let max_x = center_xz.0 + size_xz.0 * 0.5;
                let min_z = center_xz.1 - size_xz.1 * 0.5;
                let max_z = center_xz.1 + size_xz.1 * 0.5;
                let vmin = grid.world_to_voxel(Vec3::new(min_x, 0.0, min_z));
                let vmax = grid.world_to_voxel(Vec3::new(max_x, 0.0, max_z));
                let base_y =
                    (terrain_height_m(spec, center_xz.0, center_xz.1) / s).round() as i32;
                let hy = (height_m / s).ceil() as i32;
                let top_y = (base_y + hy).min(dims.y as i32 - 1);

                // Выравниваем фундамент под домом (неразрушаемый грунт).
                for z in vmin.z..=vmax.z {
                    for x in vmin.x..=vmax.x {
                        for y in 1..base_y {
                            grid.set(IVec3::new(x, y, z), Material::Soil);
                        }
                    }
                }
                // Полые стены толщиной в 1 воксель + оконные проёмы + крыша.
                for y in base_y..=top_y {
                    for z in vmin.z..=vmax.z {
                        for x in vmin.x..=vmax.x {
                            let on_x = x == vmin.x || x == vmax.x;
                            let on_z = z == vmin.z || z == vmax.z;
                            let is_roof = y == top_y;
                            if !(on_x || on_z || is_roof) {
                                continue;
                            }
                            let mid_y = base_y + hy / 2;
                            let mid_x = (vmin.x + vmax.x) / 2;
                            let mid_z = (vmin.z + vmax.z) / 2;
                            let window = (y == mid_y || y == mid_y + 1)
                                && ((on_z && (x - mid_x).abs() <= 1)
                                    || (on_x && (z - mid_z).abs() <= 1));
                            if !window {
                                let m = if is_roof { Material::Wood } else { wall_mat };
                                grid.set(IVec3::new(x, y, z), m);
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. Опорные стойки мишеней (неразрушаемые стальные тумбы).
    for t in &spec.targets {
        let v = grid.world_to_voxel(Vec3::new(t.pos_xz.0, 0.0, t.pos_xz.1));
        let base_y = (terrain_height_m(spec, t.pos_xz.0, t.pos_xz.1) / s).round() as i32;
        for dx in [-1, 1] {
            for y in base_y..(base_y + 2) {
                grid.set(IVec3::new(v.x + dx, y, v.z + 1), Material::TargetPost);
            }
        }
    }

    grid
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maps::spec::{FROZEN_FORT_RON, PROVING_GROUNDS_RON, available_maps};
    use crate::voxel::VoxelWorld;

    #[test]
    fn proving_grounds_builds_with_indestructible_ground_and_destructible_buildings() {
        let spec = MapSpec::from_ron(PROVING_GROUNDS_RON).expect("proving_grounds.ron");
        assert_eq!(spec.targets.len(), 9);
        let grid = generate_map_grid(&spec);
        assert!(grid.count_solid() > 100_000);

        // Зона спавна ровная.
        let h0 = grid
            .surface_height_at(spec.spawn_xz.0, spec.spawn_xz.1)
            .unwrap();
        let h1 = grid
            .surface_height_at(spec.spawn_xz.0 + 2.0, spec.spawn_xz.1 - 2.0)
            .unwrap();
        assert!((h0 - h1).abs() < 0.01, "h0={h0} h1={h1}");

        // Выстрел в чистый грунт ничего не удаляет.
        let mut world = VoxelWorld::new(grid);
        let res_ground = world.carve_sphere(Vec3::new(0.0, h0 - 0.5, 100.0), 1.5);
        assert_eq!(res_ground.removed.len(), 0);
        assert!(res_ground.protected_terrain > 0);

        // Выстрел в кирпичную стену (-21, h+1.5, 75.7) разрушает воксели стены, но не грунт.
        let wall_y = terrain_height_m(&spec, -21.0, 75.7);
        let res_wall = world.carve_sphere(Vec3::new(-21.0, wall_y + 1.2, 75.7), 1.2);
        assert!(!res_wall.removed.is_empty(), "стена должна разрушаться");
        assert!(
            res_wall.removed.iter().all(|(_, m)| m.is_destructible()),
            "удалены должны быть только разрушаемые материалы"
        );
    }

    #[test]
    fn all_maps_have_indestructible_ground_and_destructible_structures() {
        let maps = available_maps();
        assert_eq!(maps.len(), 10, "должно быть ровно 10 карт (Полигон + 9 боевых)");

        for spec in maps {
            assert_eq!(spec.ally_spawns.len(), 4, "карта {} без 4 союзников", spec.id);
            assert_eq!(spec.enemy_spawns.len(), 5, "карта {} без 5 врагов", spec.id);
            let grid = generate_map_grid(&spec);
            assert!(
                grid.count_solid() > 50_000,
                "слишком мало вокселей на карте {}",
                spec.id
            );

            let h0 = grid
                .surface_height_at(spec.spawn_xz.0, spec.spawn_xz.1)
                .unwrap();
            let h1 = grid
                .surface_height_at(spec.spawn_xz.0 + 2.0, spec.spawn_xz.1 - 2.0)
                .unwrap();
            assert!(
                (h0 - h1).abs() < 0.05,
                "спавн игрока на карте {} неровный: {h0} vs {h1}",
                spec.id
            );

            for (i, s_pos) in spec
                .ally_spawns
                .iter()
                .chain(spec.enemy_spawns.iter())
                .enumerate()
            {
                let sh0 = grid
                    .surface_height_at(s_pos.pos_xz.0, s_pos.pos_xz.1)
                    .unwrap();
                let sh1 = grid
                    .surface_height_at(s_pos.pos_xz.0 + 2.0, s_pos.pos_xz.1 - 2.0)
                    .unwrap();
                assert!(
                    (sh0 - sh1).abs() < 0.05,
                    "ИИ-спавн #{i} на карте {} неровный: {sh0} vs {sh1}",
                    spec.id
                );
            }

            let mut world = VoxelWorld::new(grid);
            let ground_res = world.carve_sphere(
                Vec3::new(spec.spawn_xz.0, h0 - spec.voxel_size * 0.5, spec.spawn_xz.1),
                1.5,
            );
            assert_eq!(
                ground_res.removed.len(),
                0,
                "рельеф спавна {} не должен разрушаться",
                spec.id
            );
            assert!(ground_res.protected_terrain > 0);

            let first = spec.structures.first().expect("на карте должны быть постройки");
            let probe = match *first {
                StructureSpec::Wall {
                    min_xz,
                    max_xz,
                    height_m,
                    ..
                } => {
                    let cx = (min_xz.0 + max_xz.0) * 0.5;
                    let cz = (min_xz.1 + max_xz.1) * 0.5;
                    let gy = terrain_height_m(&spec, cx, cz);
                    Vec3::new(cx, gy + height_m * 0.45, cz)
                }
                StructureSpec::House {
                    center_xz,
                    size_xz,
                    height_m,
                    ..
                } => {
                    let wx = center_xz.0 - size_xz.0 * 0.5;
                    let wz = center_xz.1;
                    let gy = terrain_height_m(&spec, wx, wz);
                    Vec3::new(wx, gy + height_m * 0.4, wz)
                }
            };
            let st_res = world.carve_sphere(probe, 1.4);
            assert!(
                !st_res.removed.is_empty(),
                "постройка на карте {} должна разрушаться",
                spec.id
            );
            assert!(
                st_res.removed.iter().all(|(_, m)| m.is_destructible()),
                "удалены должны быть только разрушаемые материалы на карте {}",
                spec.id
            );
        }

        // Проверка ледяного озера на зимней карте.
        let winter = MapSpec::from_ron(FROZEN_FORT_RON).unwrap();
        let grid = generate_map_grid(&winter);
        let ice_hit = grid.raycast(Vec3::new(0.0, 10.0, 0.0), Vec3::NEG_Y, 15.0).unwrap();
        assert_eq!(ice_hit.material, Material::Ice);
    }

    #[test]
    fn new_maps_produce_expected_biomes_and_no_range_lane() {
        use crate::maps::spec::{ALPINE_PASS_RON, NORDIC_FJORD_RON, PROKHOROVKA_RIDGE_RON};

        // Альпийский перевал: заснеженный пик в центре и зелёная долина на подходах.
        let alpine = MapSpec::from_ron(ALPINE_PASS_RON).unwrap();
        let alpine_grid = generate_map_grid(&alpine);
        let peak_hit = alpine_grid
            .raycast(Vec3::new(0.0, 27.5, 0.0), Vec3::NEG_Y, 28.0)
            .unwrap();
        assert_eq!(peak_hit.material, Material::Snow);
        let valley_hit = alpine_grid
            .raycast(Vec3::new(0.0, 10.0, 80.0), Vec3::NEG_Y, 15.0)
            .unwrap();
        assert_eq!(valley_hit.material, Material::Grass);

        // Ледяной фьорд: замёрзшее западное русло покрыто льдом.
        let fjord = MapSpec::from_ron(NORDIC_FJORD_RON).unwrap();
        let fjord_grid = generate_map_grid(&fjord);
        let fjord_ice = fjord_grid
            .raycast(Vec3::new(-66.0, 10.0, 0.0), Vec3::NEG_Y, 15.0)
            .unwrap();
        assert_eq!(fjord_ice.material, Material::Ice);

        // Прохоровская дуга: умеренный биом без песчаной стрельбищной полосы Полигона.
        let prokh = MapSpec::from_ron(PROKHOROVKA_RIDGE_RON).unwrap();
        let prokh_grid = generate_map_grid(&prokh);
        for z in [70.0, 90.0, 110.0] {
            let hit = prokh_grid
                .raycast(Vec3::new(0.0, 15.0, z), Vec3::NEG_Y, 20.0)
                .unwrap();
            assert_ne!(hit.material, Material::Sand);
        }
    }
}
