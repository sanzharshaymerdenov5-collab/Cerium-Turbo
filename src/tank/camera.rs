//! Камера игрока: орбитальная от 3-го лица + снайперский прицел (Shift) + точка прицеливания.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use super::armor::ArmorOwner;
use super::spawn::{AimCommand, GunPivot, MuzzlePoint, PlayerTank, TankRoot};
use crate::ballistics::{ArmorPlate, TargetInfo, geometry::ray_plate};
use crate::core::GameState;
use crate::voxel::VoxelWorld;

/// Состояние камеры танка.
#[derive(Component, Debug)]
pub struct TankCamera {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub sniper: bool,
}

impl Default for TankCamera {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: -0.22,
            distance: 11.5,
            sniper: false,
        }
    }
}

/// Информация о бронеплите под прицелом (для индикатора пробития).
#[derive(Debug, Clone, Copy)]
pub struct AimPlateInfo {
    pub thickness_mm: f32,
    pub angle_deg: f32,
    pub is_enemy_tank: bool,
}

/// Куда сейчас смотрит прицел камеры и куда в реальности смотрит ствол.
#[derive(Resource, Debug)]
pub struct AimTarget {
    /// Точка в мире, на которую наведён перекрест камеры.
    pub point: Vec3,
    /// Дистанция до `point` от камеры, м.
    pub distance: f32,
    /// Имя мишени под прицелом (если есть).
    pub target_name: Option<String>,
    /// Параметры плиты под прицелом (если есть).
    pub target_plate: Option<AimPlateInfo>,
    /// Точка, куда сейчас смотрит линия ствола (для маркера сведения).
    pub gun_point: Vec3,
}

impl Default for AimTarget {
    fn default() -> Self {
        Self {
            point: Vec3::new(0.0, 2.0, -100.0),
            distance: 100.0,
            target_name: None,
            target_plate: None,
            gun_point: Vec3::new(0.0, 2.0, -100.0),
        }
    }
}

pub fn spawn_battle_camera(commands: &mut Commands) {
    commands.spawn((
        Name::new("BattleCamera"),
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: 60f32.to_radians(),
            near: 0.15,
            far: 1500.0,
            ..default()
        }),
        Transform::from_xyz(0.0, 8.0, 140.0).looking_at(Vec3::new(0.0, 3.0, 100.0), Vec3::Y),
        TankCamera::default(),
        DespawnOnExit(GameState::InBattle),
    ));
}

pub fn release_cursor_on_exit(mut cursors: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    if let Ok(mut c) = cursors.single_mut() {
        c.grab_mode = CursorGrabMode::None;
        c.visible = true;
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_tank_camera(
    keys: Res<ButtonInput<KeyCode>>,
    mouse_btn: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut cursors: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mut tanks: Query<(Entity, &GlobalTransform, &mut AimCommand), (With<TankRoot>, With<PlayerTank>)>,
    guns: Query<(&GlobalTransform, &GunPivot)>,
    muzzles: Query<(&GlobalTransform, &MuzzlePoint)>,
    world: Option<Res<VoxelWorld>>,
    plates: Query<(&GlobalTransform, &ArmorPlate, &TargetInfo, Option<&ArmorOwner>)>,
    mut aim: ResMut<AimTarget>,
    mut cam_q: Query<(&mut Transform, &mut Projection, &mut TankCamera), With<Camera3d>>,
) {
    let Ok((mut cam_tf, mut proj, mut cam)) = cam_q.single_mut() else {
        return;
    };
    let Ok((player_ent, tank_gt, mut aim_cmd)) = tanks.single_mut() else {
        return;
    };

    // Захват/освобождение курсора.
    let mut grabbed = false;
    if let Ok(mut cursor) = cursors.single_mut() {
        if keys.just_pressed(KeyCode::Escape) {
            cursor.grab_mode = CursorGrabMode::None;
            cursor.visible = true;
        } else if mouse_btn.just_pressed(MouseButton::Left)
            || mouse_btn.just_pressed(MouseButton::Right)
        {
            cursor.grab_mode = CursorGrabMode::Locked;
            cursor.visible = false;
        }
        grabbed = cursor.grab_mode != CursorGrabMode::None
            || mouse_btn.pressed(MouseButton::Right);
    }

    if keys.just_pressed(KeyCode::ShiftLeft) || keys.just_pressed(KeyCode::ShiftRight) {
        cam.sniper = !cam.sniper;
    }

    let sens = if cam.sniper { 0.0012 } else { 0.0026 };
    if grabbed && motion.delta != Vec2::ZERO {
        cam.yaw -= motion.delta.x * sens;
        cam.pitch = (cam.pitch - motion.delta.y * sens).clamp(-1.15, 0.45);
    }
    if scroll.delta.y.abs() > 0.0 && !cam.sniper {
        cam.distance = (cam.distance - scroll.delta.y * 0.9).clamp(5.0, 24.0);
    }

    let rot = Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);
    let forward = rot * Vec3::NEG_Z;

    if cam.sniper {
        let anchor = guns
            .iter()
            .find(|(_, g)| g.tank == player_ent)
            .map(|(g, _)| g.translation() + g.up() * 0.25 + g.forward() * 0.6)
            .unwrap_or_else(|| tank_gt.translation() + Vec3::new(0.0, 2.1, 0.0));
        cam_tf.translation = anchor;
        cam_tf.rotation = rot;
        if let Projection::Perspective(ref mut p) = *proj {
            p.fov = 20f32.to_radians();
        }
    } else {
        let focus = tank_gt.translation() + Vec3::new(0.0, 2.2, 0.0);
        let desired = focus - forward * cam.distance;
        let mut pos = desired;
        if let Some(w) = &world {
            if let Some(h) = w.grid.surface_height_at(pos.x, pos.z) {
                pos.y = pos.y.max(h + 0.6);
            }
        }
        cam_tf.translation = pos;
        cam_tf.rotation = rot;
        if let Projection::Perspective(ref mut p) = *proj {
            p.fov = 60f32.to_radians();
        }
    }

    let (pt, dist, name, plate_info) = raycast_aim(
        cam_tf.translation,
        forward,
        if cam.sniper { 0.4 } else { (cam.distance - 2.0).max(0.5) },
        Some(player_ent),
        world.as_deref(),
        &plates,
    );
    aim.point = pt;
    aim.distance = dist;
    aim.target_name = name;
    aim.target_plate = plate_info;
    aim_cmd.world_point = pt;

    if let Some((m_gt, _)) = muzzles.iter().find(|(_, m)| m.tank == player_ent) {
        let m_pos = m_gt.translation();
        let m_dir = m_gt.forward().as_vec3();
        let (gpt, _, _, _) = raycast_aim(
            m_pos,
            m_dir,
            0.2,
            Some(player_ent),
            world.as_deref(),
            &plates,
        );
        aim.gun_point = gpt;
    } else {
        aim.gun_point = aim.point;
    }
}

fn raycast_aim(
    origin: Vec3,
    dir: Vec3,
    skip: f32,
    ignore_tank: Option<Entity>,
    world: Option<&VoxelWorld>,
    plates: &Query<(&GlobalTransform, &ArmorPlate, &TargetInfo, Option<&ArmorOwner>)>,
) -> (Vec3, f32, Option<String>, Option<AimPlateInfo>) {
    let max_d = 800.0;
    let start = origin + dir * skip;
    let mut best_t = max_d;
    let mut name = None;
    let mut plate_info = None;
    if let Some(w) = world {
        if let Some(h) = w.grid.raycast(start, dir, max_d) {
            best_t = h.distance;
        }
    }
    for (gt, plate, info, owner) in plates {
        if owner.is_some_and(|o| Some(o.tank) == ignore_tank) {
            continue;
        }
        let (_, rot, pos) = gt.to_scale_rotation_translation();
        if let Some(h) = ray_plate(start, dir, best_t, pos, rot, plate.half_extents) {
            best_t = h.t;
            name = Some(info.name.clone());
            plate_info = Some(AimPlateInfo {
                thickness_mm: plate.thickness_mm,
                angle_deg: h.angle_deg,
                is_enemy_tank: owner.is_some(),
            });
        }
    }
    let total = skip + best_t;
    (origin + dir * total, total, name, plate_info)
}
