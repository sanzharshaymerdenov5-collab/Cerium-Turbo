//! Модуль карт: загрузка `.ron`, генерация воксельного мира, спавн мишеней и освещения.

pub mod generate;
pub mod spec;

use avian3d::prelude::*;
use bevy::math::{IVec3, UVec3};
use bevy::prelude::*;

use crate::ai::{AiBrain, VoxelNavGrid};
use crate::ballistics::{ArmorPlate, HitLog, TargetInfo};
use crate::core::{BattleSetupSet, BattleStatus, GameState, SelectedMap, SelectedTank, Team};
use crate::damage::{DamageFeed, XRayHitReport};
use crate::tank::spawn::spawn_tank_with_team;
use crate::tank::{load_tank_spec, spawn_battle_camera};
use crate::voxel::{
    ChunkEntities, Material, VoxelAssets, VoxelGrid, VoxelWorld, spawn_world_chunks,
    voxel_model_bundle,
};
pub use generate::{generate_map_grid, terrain_height_m};
pub use spec::{
    CITY_RUINS_RON, DESERT_CANYON_RON, EnemySpawnSpec, FROZEN_FORT_RON, MapBiome, MapSpec,
    PROVING_GROUNDS_RON, RAIL_YARD_RON, RANDOM_MAP_ID, TargetSpec, available_maps, load_map_spec,
    resolve_map_spec,
};

/// Итоговая карта текущего боя (с учётом режима выбора «Случайная»).
#[derive(Resource, Debug, Clone)]
pub struct ActiveBattleMap(pub MapSpec);

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(GameState::InBattle),
            (
                resolve_battle_map.in_set(BattleSetupSet::ResolveMap),
                setup_battle_scene.in_set(BattleSetupSet::SpawnWorld),
            ),
        );
    }
}

fn resolve_battle_map(
    mut commands: Commands,
    selected_map: Res<SelectedMap>,
    time: Res<Time>,
    mut battle_counter: Local<u64>,
) {
    *battle_counter = battle_counter.wrapping_add(1);
    let seed = (time.elapsed().as_nanos() as u64)
        ^ battle_counter.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let map_spec = resolve_map_spec(&selected_map.0, seed);
    commands.insert_resource(ActiveBattleMap(map_spec));
}

fn build_target_plate_grid(size_xy: (f32, f32)) -> VoxelGrid {
    let vs = 0.15;
    let nx = ((size_xy.0 / vs).ceil() as u32).max(2);
    let ny = ((size_xy.1 / vs).ceil() as u32).max(2);
    let dims = UVec3::new(nx, ny, 2);
    let origin = Vec3::new(
        -(nx as f32) * vs * 0.5,
        -(ny as f32) * vs * 0.5,
        -vs,
    );
    let mut g = VoxelGrid::new(dims, vs, origin);
    g.fill_box(
        IVec3::ZERO,
        IVec3::new(nx as i32 - 1, ny as i32 - 1, 1),
        Material::TargetSteel,
    );
    let cx = nx as i32 / 2;
    let cy = ny as i32 / 2;
    for dy in -1..=1 {
        for dx in -1..=1 {
            g.set(IVec3::new(cx + dx, cy + dy, 1), Material::TankStar);
        }
    }
    g
}

#[allow(clippy::too_many_arguments)]
fn setup_battle_scene(
    mut commands: Commands,
    active_map: Option<Res<ActiveBattleMap>>,
    selected_map: Res<SelectedMap>,
    selected_tank: Res<SelectedTank>,
    mut meshes: ResMut<Assets<Mesh>>,
    assets: Res<VoxelAssets>,
    mut chunks: ResMut<ChunkEntities>,
    mut log: ResMut<HitLog>,
    mut feed: ResMut<DamageFeed>,
    mut xray: ResMut<XRayHitReport>,
    mut status: ResMut<BattleStatus>,
) {
    *log = HitLog::default();
    *feed = DamageFeed::default();
    *xray = XRayHitReport::default();
    *status = BattleStatus::Ongoing;

    let map_spec = active_map
        .map(|m| m.0.clone())
        .unwrap_or_else(|| resolve_map_spec(&selected_map.0, 1));
    let grid = generate_map_grid(&map_spec);
    let nav_grid = VoxelNavGrid::from_grid(&grid, VoxelNavGrid::DEFAULT_CELL_M);
    let world = VoxelWorld::new(grid);

    spawn_world_chunks(&mut commands, &world, &mut meshes, &assets, &mut chunks);

    // Освещение и небо по биому.
    let sky = match map_spec.biome {
        MapBiome::Temperate => Color::srgb(0.58, 0.72, 0.88),
        MapBiome::City => Color::srgb(0.48, 0.54, 0.62),
        MapBiome::Desert => Color::srgb(0.84, 0.78, 0.65),
        MapBiome::Winter => Color::srgb(0.74, 0.80, 0.88),
        MapBiome::Industrial => Color::srgb(0.45, 0.49, 0.55),
    };
    commands.insert_resource(ClearColor(sky));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.85, 0.90, 1.0),
        brightness: 380.0,
        ..default()
    });
    commands.spawn((
        Name::new("SunLight"),
        DirectionalLight {
            illuminance: 11_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(60.0, 120.0, 80.0).looking_at(Vec3::ZERO, Vec3::Y),
        DespawnOnExit(GameState::InBattle),
    ));

    // Бронеплиты-мишени.
    for t in &map_spec.targets {
        let ground_y = terrain_height_m(&map_spec, t.pos_xz.0, t.pos_xz.1);
        let center = Vec3::new(t.pos_xz.0, ground_y + t.size_xy.1 * 0.5 + 0.35, t.pos_xz.1);
        let rot = Quat::from_rotation_x(-t.slope_deg.to_radians());
        let plate_grid = build_target_plate_grid(t.size_xy);
        let vis = commands
            .spawn(voxel_model_bundle(&plate_grid, &mut meshes, &assets))
            .id();
        commands
            .spawn((
                Name::new(format!("Target {}", t.name)),
                Transform::from_translation(center).with_rotation(rot),
                Visibility::default(),
                ArmorPlate {
                    thickness_mm: t.thickness_mm,
                    half_extents: Vec2::new(t.size_xy.0 * 0.5, t.size_xy.1 * 0.5),
                },
                TargetInfo {
                    name: t.name.clone(),
                },
                RigidBody::Static,
                Collider::cuboid(t.size_xy.0, t.size_xy.1, 0.30),
                DespawnOnExit(GameState::InBattle),
            ))
            .add_child(vis);
    }

    // Танк игрока и камера.
    let tank_spec = load_tank_spec(&selected_tank.0.0)
        .unwrap_or_else(|| load_tank_spec("t34_85").expect("t34_85"));
    let spawn_y = world
        .grid
        .surface_height_at(map_spec.spawn_xz.0, map_spec.spawn_xz.1)
        .unwrap_or(map_spec.base_height_m)
        + 0.35;
    let spawn_pos = Vec3::new(map_spec.spawn_xz.0, spawn_y, map_spec.spawn_xz.1);
    spawn_tank_with_team(
        &mut commands,
        &mut meshes,
        &assets,
        &tank_spec,
        spawn_pos,
        map_spec.spawn_yaw_deg.to_radians(),
        Team::Player,
        true,
        0,
    );
    spawn_battle_camera(&mut commands);

    // Патрульные точки на карте.
    let waypoints: Vec<Vec3> = map_spec
        .waypoints
        .iter()
        .map(|&(wx, wz)| {
            let wy = world
                .grid
                .surface_height_at(wx, wz)
                .unwrap_or(map_spec.base_height_m)
                + 0.3;
            Vec3::new(wx, wy, wz)
        })
        .collect();

    // Союзные ИИ-танки (Team::Player, без PlayerTank).
    for (idx, als) in map_spec.ally_spawns.iter().enumerate() {
        let a_spec = als
            .tank_id
            .as_deref()
            .and_then(load_tank_spec)
            .unwrap_or_else(|| load_tank_spec("t34_85").expect("t34_85"));
        let ay = world
            .grid
            .surface_height_at(als.pos_xz.0, als.pos_xz.1)
            .unwrap_or(map_spec.base_height_m)
            + 0.35;
        let apos = Vec3::new(als.pos_xz.0, ay, als.pos_xz.1);
        let ally_ent = spawn_tank_with_team(
            &mut commands,
            &mut meshes,
            &assets,
            &a_spec,
            apos,
            als.yaw_deg.to_radians(),
            Team::Player,
            false,
            (idx as u64) + 1,
        );
        let mut bot_wps = waypoints.clone();
        if !bot_wps.is_empty() {
            let rot_by = idx % bot_wps.len();
            bot_wps.rotate_left(rot_by);
        }
        commands.entity(ally_ent).insert(AiBrain::new(
            bot_wps,
            9001 + (idx as u64 + 1) * 131,
            apos,
        ));
    }

    // Вражеские ИИ-танки.
    for (idx, es) in map_spec.enemy_spawns.iter().enumerate() {
        let e_spec = es
            .tank_id
            .as_deref()
            .and_then(load_tank_spec)
            .unwrap_or_else(|| load_tank_spec("t34_85").expect("t34_85"));
        let ey = world
            .grid
            .surface_height_at(es.pos_xz.0, es.pos_xz.1)
            .unwrap_or(map_spec.base_height_m)
            + 0.35;
        let epos = Vec3::new(es.pos_xz.0, ey, es.pos_xz.1);
        let enemy_ent = spawn_tank_with_team(
            &mut commands,
            &mut meshes,
            &assets,
            &e_spec,
            epos,
            es.yaw_deg.to_radians(),
            Team::Enemy,
            false,
            (idx as u64) + 10,
        );
        let mut bot_wps = waypoints.clone();
        if !bot_wps.is_empty() {
            let rot_by = idx % bot_wps.len();
            bot_wps.rotate_left(rot_by);
        }
        commands.entity(enemy_ent).insert(AiBrain::new(
            bot_wps,
            9001 + (idx as u64 + 10) * 131,
            epos,
        ));
    }

    commands.insert_resource(nav_grid);
    commands.insert_resource(world);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::ai::has_line_of_sight;
    use crate::tank::TankState;
    use crate::tank::controller::tank_suspension_and_drive;
    use bevy::state::app::StatesPlugin;
    use bevy::time::TimeUpdateStrategy;

    #[test]
    fn resolve_map_spec_random_selects_valid_maps_across_seeds() {
        let valid_ids: Vec<String> = available_maps().into_iter().map(|m| m.id).collect();
        let mut seen = std::collections::HashSet::new();
        for seed in 0..16u64 {
            let m = resolve_map_spec(RANDOM_MAP_ID, seed);
            assert!(
                valid_ids.contains(&m.id),
                "некорректный id случайной карты: {}",
                m.id
            );
            seen.insert(m.id);
        }
        assert!(
            seen.len() >= 2,
            "случайный выбор карты должен давать разные локации: {seen:?}"
        );
        assert_eq!(resolve_map_spec("city_ruins", 5).id, "city_ruins");
    }

    use crate::tank::spawn::spawn_tank;

    #[test]
    fn all_five_maps_enemy_spawns_are_reachable_on_nav_grid() {
        let maps = available_maps();
        assert_eq!(maps.len(), 5);
        for spec in maps {
            let grid = generate_map_grid(&spec);
            let nav = VoxelNavGrid::from_grid(&grid, VoxelNavGrid::DEFAULT_CELL_M);
            let p = Vec3::new(spec.spawn_xz.0, spec.base_height_m, spec.spawn_xz.1);
            assert_eq!(spec.ally_spawns.len(), 4, "карта {} без 4 союзников", spec.id);
            assert_eq!(spec.enemy_spawns.len(), 5, "карта {} без 5 врагов", spec.id);
            for als in &spec.ally_spawns {
                let ap = Vec3::new(als.pos_xz.0, spec.base_height_m, als.pos_xz.1);
                let path = nav.find_path(p, ap);
                assert!(
                    !path.is_empty(),
                    "на карте {} спавн союзника {:?} должен быть достижим от игрока {:?}",
                    spec.id,
                    als.pos_xz,
                    spec.spawn_xz
                );
            }
            for es in &spec.enemy_spawns {
                let ep = Vec3::new(es.pos_xz.0, spec.base_height_m, es.pos_xz.1);
                let path = nav.find_path(p, ep);
                assert!(
                    !path.is_empty(),
                    "на карте {} спавн врага {:?} должен быть достижим от игрока {:?}",
                    spec.id,
                    es.pos_xz,
                    spec.spawn_xz
                );
            }
        }
    }

    #[test]
    fn all_five_maps_5v5_spawns_budget_and_zero_spawn_los() {
        let maps = available_maps();
        assert_eq!(maps.len(), 5);
        for spec in maps {
            assert_eq!(spec.ally_spawns.len(), 4, "карта {} без 4 союзников", spec.id);
            assert_eq!(spec.enemy_spawns.len(), 5, "карта {} без 5 врагов", spec.id);
            assert!(
                spec.dims.1 <= 36,
                "высота сетки {} превышает 36: {}",
                spec.id,
                spec.dims.1
            );
            let total_voxels =
                (spec.dims.0 as usize) * (spec.dims.1 as usize) * (spec.dims.2 as usize);
            assert!(
                total_voxels <= 3_200_000,
                "бюджет вокселей {} превышен: {}",
                spec.id,
                total_voxels
            );

            let grid = generate_map_grid(&spec);
            let nav = VoxelNavGrid::from_grid(&grid, VoxelNavGrid::DEFAULT_CELL_M);

            let mut friend_pts = vec![spec.spawn_xz];
            friend_pts.extend(spec.ally_spawns.iter().map(|s| s.pos_xz));
            let enemy_pts: Vec<(f32, f32)> = spec.enemy_spawns.iter().map(|s| s.pos_xz).collect();

            for &(fx, fz) in &friend_pts {
                let fh = grid.surface_height_at(fx, fz).unwrap_or(spec.base_height_m);
                let fpos = Vec3::new(fx, fh + 0.35, fz);
                for &(ex, ez) in &enemy_pts {
                    let eh = grid.surface_height_at(ex, ez).unwrap_or(spec.base_height_m);
                    let epos = Vec3::new(ex, eh + 0.35, ez);
                    assert!(
                        !has_line_of_sight(&grid, fpos + Vec3::Y * 2.2, epos + Vec3::Y * 2.2),
                        "на карте {} открыта прямая видимость между ({fx},{fz}) и ({ex},{ez})",
                        spec.id
                    );
                    assert!(
                        !nav.find_path(fpos, epos).is_empty(),
                        "на карте {} нет A*-пути между ({fx},{fz}) и ({ex},{ez})",
                        spec.id
                    );
                }
            }
        }
    }

    #[test]
    fn frozen_fort_ice_lake_reduces_track_traction() {
        let spec = MapSpec::from_ron(FROZEN_FORT_RON).unwrap();
        let grid = generate_map_grid(&spec);
        let h = grid.surface_height_at(0.0, 0.0).unwrap();

        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            StatesPlugin,
            PhysicsPlugins::default(),
        ))
        .init_state::<GameState>()
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_resource::<VoxelAssets>()
        .insert_resource(VoxelWorld::new(grid))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )))
        .add_systems(FixedUpdate, tank_suspension_and_drive);

        app.world_mut()
            .resource_mut::<NextState<GameState>>()
            .set(GameState::InBattle);

        app.add_systems(
            Startup,
            move |mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, assets: Res<VoxelAssets>| {
                let tank = load_tank_spec("t34_85").unwrap();
                spawn_tank(
                    &mut commands,
                    &mut meshes,
                    &assets,
                    tank,
                    Vec3::new(0.0, h + 0.25, 0.0),
                    0.0,
                    true,
                );
            },
        );
        app.finish();
        app.cleanup();

        for _ in 0..20 {
            app.update();
        }

        let mut q = app.world_mut().query::<&TankState>();
        let st = q.single(app.world()).unwrap();
        assert!(st.grounded_wheels >= 4, "катки должны опираться на лёд: {}", st.grounded_wheels);
        assert!(
            st.surface_traction <= 0.30,
            "сцепление на льду должно падать до <=0.30, получено {}",
            st.surface_traction
        );
    }
}
