//! Спавн иерархии танка (корпус → башня → орудие → дульный срез).

use avian3d::prelude::*;
use bevy::prelude::*;

use super::armor::{TankCombat, spawn_tank_armor_plates};
use super::builder::{build_tank_parts_for_team, TankVoxelParts};
use super::spec::{TankSpec, VoxelVisualSource};
use crate::core::{GameState, Team};
use crate::damage::{TankCrew, TankHealth, TankModules};
use crate::voxel::{VoxelAssets, voxel_model_bundle, vox_loader::load_vox};

/// Целевая мировая точка наведения орудия (для игрока и ИИ).
#[derive(Component, Debug, Clone, Copy)]
pub struct AimCommand {
    pub world_point: Vec3,
}

/// Корень физического танка.
#[derive(Component, Debug, Clone)]
pub struct TankRoot {
    pub entity: Entity,
    pub spec: TankSpec,
    pub spawn_pos: Vec3,
    pub spawn_yaw: f32,
}

/// Маркер танка игрока.
#[derive(Component, Debug, Default)]
pub struct PlayerTank;

/// Текущее боевое/ходовое состояние танка.
#[derive(Component, Debug, Default)]
pub struct TankState {
    /// Остаток перезарядки, с (0 = заряжено).
    pub reload_timer: f32,
    /// Текущая продольная скорость, км/ч (со знаком).
    pub speed_kmh: f32,
    /// Сколько катков сейчас опираются на грунт.
    pub grounded_wheels: u8,
    /// Коэффициент сцепления с грунтом под гусеницами (1.0 = земля, 0.25 = лёд).
    pub surface_traction: f32,
    pub throttle: f32,
    pub steer: f32,
    pub fire_queued: bool,
    pub reset_queued: bool,
}

/// Точка крепления луча подвески (в локальном кадре `TankRoot`).
#[derive(Debug, Clone, Copy)]
pub struct WheelMount {
    pub local_pos: Vec3,
    /// −1 = левый борт, +1 = правый борт.
    pub side: f32,
}

/// 8-точечная пружинно-демпферная подвеска.
#[derive(Component, Debug, Clone)]
pub struct Suspension {
    pub wheels: Vec<WheelMount>,
    /// Высота крепления луча над номинальной точкой касания грунта, м.
    pub rest_length: f32,
    pub max_drop: f32,
    pub spring_k: f32,
    pub damper_c: f32,
    pub max_force: f32,
}

impl Suspension {
    pub fn for_spec(spec: &TankSpec) -> Self {
        let p = &spec.profile;
        let x = p.hull_width * 0.5 - p.track_width * 0.5;
        let y = 0.60;
        let zs = [-0.72, -0.24, 0.24, 0.72];
        let mut wheels = Vec::with_capacity(8);
        for side in [-1.0f32, 1.0] {
            for kz in zs {
                wheels.push(WheelMount {
                    local_pos: Vec3::new(side * x, y, kz * p.hull_length * 0.5),
                    side,
                });
            }
        }
        // На 8 катков в покое приходится m*g; при просадке ~0.15 м k ≈ m*g / (8*0.15).
        let weight_per_wheel = spec.mass_kg * 9.81 / 8.0;
        let spring_k = weight_per_wheel / 0.15;
        let damper_c = 2.0 * (spring_k * (spec.mass_kg / 8.0)).sqrt() * 0.85;
        Self {
            wheels,
            rest_length: y + 0.15,
            max_drop: y + 0.35,
            spring_k,
            damper_c,
            max_force: weight_per_wheel * 4.5,
        }
    }
}

/// Узел вращения башни (yaw вокруг +Y).
#[derive(Component, Debug, Clone, Copy)]
pub struct TurretPivot {
    pub tank: Entity,
}

/// Узел вертикального наведения орудия (pitch вокруг +X).
#[derive(Component, Debug, Clone, Copy)]
pub struct GunPivot {
    pub tank: Entity,
}

/// Точка вылета снаряда на дульном срезе.
#[derive(Component, Debug, Clone, Copy)]
pub struct MuzzlePoint {
    pub tank: Entity,
}

/// Меш ствола (используется для анимации отката и наката при выстреле).
#[derive(Component, Debug, Clone, Copy)]
pub struct BarrelMesh {
    pub tank: Entity,
    pub rest_translation: Vec3,
}

/// Выпуклый коллайдер корпуса с приподнятым днищем и скошенными носом/кормой
/// (чтобы подвеска несла танк, а нос не цеплялся за ступеньки вокселей).
pub fn hull_collider(spec: &TankSpec) -> Collider {
    let hw = spec.profile.hull_width * 0.48;
    let hl = spec.profile.hull_length * 0.48;
    let top = spec.profile.deck_height + 0.35;
    let belly = 0.48;
    let nose_y = 0.78;
    let nose_z = hl * 0.78;
    let pts = [
        // Днище (укороченное по Z).
        Vec3::new(-hw, belly, -nose_z),
        Vec3::new(hw, belly, -nose_z),
        Vec3::new(-hw, belly, nose_z),
        Vec3::new(hw, belly, nose_z),
        // Скошенные нос и корма.
        Vec3::new(-hw, nose_y, -hl),
        Vec3::new(hw, nose_y, -hl),
        Vec3::new(-hw, nose_y, hl),
        Vec3::new(hw, nose_y, hl),
        // Верх корпуса.
        Vec3::new(-hw, top, -hl * 0.9),
        Vec3::new(hw, top, -hl * 0.9),
        Vec3::new(-hw, top, hl * 0.9),
        Vec3::new(hw, top, hl * 0.9),
    ];
    Collider::convex_hull(&pts).unwrap_or_else(|| Collider::cuboid(hw * 2.0, top - belly, hl * 2.0))
}

/// Спавнит боевой танк с физикой Avian3D, бронеплитами и иерархией воксельных мешей.
pub fn spawn_tank(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    assets: &VoxelAssets,
    spec: TankSpec,
    pos: Vec3,
    yaw: f32,
    is_player: bool,
) -> Entity {
    let team = if is_player { Team::Player } else { Team::Enemy };
    let parts = build_visual_parts(&spec, team);
    let susp = Suspension::for_spec(&spec);
    let col = hull_collider(&spec);
    let turret_offset = Vec3::from_array(spec.profile.turret_pivot);
    let gun_offset = Vec3::from_array(spec.profile.gun_pivot);
    let muzzle_offset = Vec3::new(0.0, 0.0, -(spec.profile.barrel_length + 0.15));
    let initial_aim = pos + Quat::from_rotation_y(yaw) * Vec3::new(0.0, 1.8, -100.0);

    let root = commands
        .spawn((
            Name::new(format!("Tank {} ({:?})", spec.name, team)),
            Transform::from_translation(pos).with_rotation(Quat::from_rotation_y(yaw)),
            Visibility::default(),
            RigidBody::Dynamic,
            col,
            Friction::new(0.02),
            Restitution::new(0.0),
            Mass(spec.mass_kg),
            CenterOfMass(Vec3::new(0.0, 0.55, 0.0)),
            LinearDamping(0.06),
            AngularDamping(0.95),
            TransformInterpolation,
            TankState {
                surface_traction: 1.0,
                ..default()
            },
            susp,
            DespawnOnExit(GameState::InBattle),
        ))
        .id();

    let combat = TankCombat::new(team, spec.name.clone());
    let health = TankHealth::new(spec.max_hp);
    let modules = TankModules::for_spec(&spec);
    let crew = TankCrew::for_spec(&spec);
    commands.entity(root).insert((
        TankRoot {
            entity: root,
            spec: spec.clone(),
            spawn_pos: pos,
            spawn_yaw: yaw,
        },
        team,
        combat,
        health,
        modules,
        crew,
        AimCommand {
            world_point: initial_aim,
        },
    ));
    if is_player {
        commands.entity(root).insert(PlayerTank);
    }

    let hull_vis = commands
        .spawn((
            Name::new("HullMesh"),
            voxel_model_bundle(&parts.hull, meshes, assets),
        ))
        .id();
    let turret_vis = commands
        .spawn((
            Name::new("TurretMesh"),
            voxel_model_bundle(&parts.turret, meshes, assets),
        ))
        .id();
    let barrel_vis = commands
        .spawn((
            Name::new("BarrelMesh"),
            BarrelMesh {
                tank: root,
                rest_translation: parts.barrel.origin,
            },
            voxel_model_bundle(&parts.barrel, meshes, assets),
        ))
        .id();
    let muzzle = commands
        .spawn((
            Name::new("MuzzlePoint"),
            MuzzlePoint { tank: root },
            Transform::from_translation(muzzle_offset),
            Visibility::default(),
        ))
        .id();
    let gun_pivot = commands
        .spawn((
            Name::new("GunPivot"),
            GunPivot { tank: root },
            Transform::from_translation(gun_offset),
            Visibility::default(),
        ))
        .add_children(&[barrel_vis, muzzle])
        .id();
    let turret_pivot = commands
        .spawn((
            Name::new("TurretPivot"),
            TurretPivot { tank: root },
            Transform::from_translation(turret_offset),
            Visibility::default(),
        ))
        .add_children(&[turret_vis, gun_pivot])
        .id();

    commands
        .entity(root)
        .add_children(&[hull_vis, turret_pivot]);

    spawn_tank_armor_plates(commands, root, turret_pivot, &spec);

    root
}

/// Маркер 3D-макета танка в ангаре (для переспавна при смене танка).
#[derive(Component, Debug, Default)]
pub struct HangarPreviewTank;

/// Спавнит визуальный макет танка без физики (для ангара).
pub fn spawn_preview_tank(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    assets: &VoxelAssets,
    spec: &TankSpec,
) -> Entity {
    let parts = build_visual_parts(spec, Team::Player);
    let turret_offset = Vec3::from_array(spec.profile.turret_pivot);
    let gun_offset = Vec3::from_array(spec.profile.gun_pivot);
    let turret_yaw = if spec.yaw_limits_deg.is_some() { 0.0 } else { 0.25 };

    let hull_vis = commands
        .spawn(voxel_model_bundle(&parts.hull, meshes, assets))
        .id();
    let turret_vis = commands
        .spawn(voxel_model_bundle(&parts.turret, meshes, assets))
        .id();
    let barrel_vis = commands
        .spawn(voxel_model_bundle(&parts.barrel, meshes, assets))
        .id();
    let gun = commands
        .spawn((Transform::from_translation(gun_offset), Visibility::default()))
        .add_child(barrel_vis)
        .id();
    let turret = commands
        .spawn((
            Transform::from_translation(turret_offset)
                .with_rotation(Quat::from_rotation_y(turret_yaw)),
            Visibility::default(),
        ))
        .add_children(&[turret_vis, gun])
        .id();

    commands
        .spawn((
            Name::new("HangarPreviewTank"),
            HangarPreviewTank,
            Transform::IDENTITY,
            Visibility::default(),
            DespawnOnExit(GameState::Hangar),
        ))
        .add_children(&[hull_vis, turret])
        .id()
}

fn build_visual_parts(spec: &TankSpec, team: Team) -> TankVoxelParts {
    let mut parts = build_tank_parts_for_team(&spec.profile, team);
    if let VoxelVisualSource::VoxFile(ref bytes_hint) = spec.visual {
        // Если передан встроенный .vox, подменяем корпус; при ошибке остаётся процедурный.
        if let Ok(grid) = load_vox(bytes_hint.as_bytes(), spec.profile.voxel_size) {
            parts.hull = grid;
        }
    }
    parts
}
