//! Модуль карт: загрузка `.ron`, генерация воксельного мира, спавн мишеней и освещения.

pub mod generate;
pub mod spec;

use avian3d::prelude::*;
use bevy::math::{IVec3, UVec3};
use bevy::prelude::*;

use crate::ballistics::{ArmorPlate, HitLog, TargetInfo};
use crate::core::{GameState, SelectedMap, SelectedTank};
use crate::tank::{load_tank_spec, spawn_battle_camera, spawn_tank};
use crate::voxel::{
    ChunkEntities, Material, VoxelAssets, VoxelGrid, VoxelWorld, spawn_world_chunks,
    voxel_model_bundle,
};
pub use generate::{generate_map_grid, terrain_height_m};
pub use spec::{MapSpec, PROVING_GROUNDS_RON, TargetSpec, load_map_spec};

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::InBattle), setup_battle_scene);
    }
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
    // Контрастная мишень в центре плиты.
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
    selected_map: Res<SelectedMap>,
    selected_tank: Res<SelectedTank>,
    mut meshes: ResMut<Assets<Mesh>>,
    assets: Res<VoxelAssets>,
    mut chunks: ResMut<ChunkEntities>,
    mut log: ResMut<HitLog>,
) {
    *log = HitLog::default();

    let map_spec = load_map_spec(&selected_map.0)
        .unwrap_or_else(|| MapSpec::from_ron(PROVING_GROUNDS_RON).expect("default map"));
    let grid = generate_map_grid(&map_spec);
    let world = VoxelWorld::new(grid);

    spawn_world_chunks(&mut commands, &world, &mut meshes, &assets, &mut chunks);

    // Освещение и небо.
    commands.insert_resource(ClearColor(Color::srgb(0.58, 0.72, 0.88)));
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

    // Бронеплиты-мишени (воксельный меш + аналитическая плита ArmorPlate + коллайдер).
    for t in &map_spec.targets {
        let ground_y = terrain_height_m(&map_spec, t.pos_xz.0, t.pos_xz.1);
        let center = Vec3::new(t.pos_xz.0, ground_y + t.size_xy.1 * 0.5 + 0.35, t.pos_xz.1);
        // Наклон назад от стрелка (стрелок на +Z, стреляет вдоль −Z; локальная +Z плиты смотрит на +Z,
        // поворот вокруг −X поднимает нормаль вверх на slope_deg).
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
    spawn_tank(
        &mut commands,
        &mut meshes,
        &assets,
        tank_spec,
        spawn_pos,
        map_spec.spawn_yaw_deg.to_radians(),
        true,
    );
    spawn_battle_camera(&mut commands);

    commands.insert_resource(world);
}
