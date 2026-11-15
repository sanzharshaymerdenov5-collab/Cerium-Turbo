//! Камера игрока: орбитальная от 3-го лица + снайперский прицел (Shift) + точка прицеливания.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use super::armor::ArmorOwner;
use super::spawn::{AimCommand, GunPivot, MuzzlePoint, PlayerTank, TankRoot, TurretPivot};
use super::turret::compute_gun_and_muzzle_world;
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
            pitch: -0.12,
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
    /// Актуальная позиция дульного среза в мире.
    pub muzzle_origin: Vec3,
    /// Актуальное направление оси канала ствола в мире.
    pub muzzle_dir: Vec3,
}

impl Default for AimTarget {
    fn default() -> Self {
        Self {
            point: Vec3::new(0.0, 2.0, -100.0),
            distance: 100.0,
            target_name: None,
            target_plate: None,
            gun_point: Vec3::new(0.0, 2.0, -100.0),
            muzzle_origin: Vec3::new(0.0, 2.0, 0.0),
            muzzle_dir: Vec3::NEG_Z,
        }
    }
}

impl AimTarget {
    /// Проверяет сведение линии ствола с точкой прицеливания по угловому расхождению
    /// (работает одинаково точно как на близкой цели, так и на горизонте на 800 м).
    pub fn is_converged(&self) -> bool {
        if self.point.distance(self.gun_point) < 0.8 {
            return true;
        }
        let Some(to_aim) = (self.point - self.muzzle_origin).try_normalize() else {
            return true;
        };
        let Some(muzzle_fwd) = self.muzzle_dir.try_normalize() else {
            return false;
        };
        muzzle_fwd.dot(to_aim) >= 0.0045f32.cos()
    }
}

/// Импульсная тряска камеры при выстреле, попадании по танку или близком подрыве БК.
#[derive(Resource, Debug, Default)]
pub struct CameraShake {
    /// Амплитуда тряски (`0..1.5`).
    pub trauma: f32,
    pub kick_dir: Vec3,
    pub elapsed: f32,
}

impl CameraShake {
    pub fn add_trauma(&mut self, amount: f32, dir: Vec3) {
        self.trauma = (self.trauma + amount.max(0.0)).min(1.4);
        if dir.length_squared() > 1e-4 {
            self.kick_dir = (self.kick_dir * 0.4 + dir.normalize() * amount).normalize_or_zero();
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
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse_btn: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut cursors: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mut tanks: Query<(Entity, &GlobalTransform, &mut AimCommand), (With<TankRoot>, With<PlayerTank>)>,
    guns: Query<(&GlobalTransform, &GunPivot)>,
    world: Option<Res<VoxelWorld>>,
    plates: Query<(&GlobalTransform, &ArmorPlate, &TargetInfo, Option<&ArmorOwner>)>,
    mut aim: ResMut<AimTarget>,
    mut shake: Option<ResMut<CameraShake>>,
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
        let sight_up = tank_gt.up().as_vec3();
        let sight_fwd = (forward - sight_up * forward.dot(sight_up))
            .normalize_or(tank_gt.forward().as_vec3());
        let anchor = guns
            .iter()
            .find(|(_, g)| g.tank == player_ent)
            .map(|(g, _)| g.translation() + sight_up * 0.24 + sight_fwd * 0.62)
            .unwrap_or_else(|| tank_gt.translation() + Vec3::new(0.0, 2.1, 0.0));
        cam_tf.translation = anchor;
        cam_tf.rotation = rot;
        if let Projection::Perspective(ref mut p) = *proj {
            p.fov = 20f32.to_radians();
        }
    } else {
        let focus = tank_gt.translation() + Vec3::new(0.0, 2.65, 0.0);
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

    // Применяем визуальную тряску камеры ПОСЛЕ вычисления линии прицеливания, чтобы не сбивать наводку.
    if let Some(ref mut s) = shake {
        let dt = time.delta_secs();
        s.elapsed += dt;
        if s.trauma > 1e-4 {
            let amp = (s.trauma * s.trauma).min(1.0);
            let t = s.elapsed;
            let scope_scale = if cam.sniper { 0.25 } else { 1.0 };
            let jitter_pos = Vec3::new(
                (t * 53.0).sin() * 0.24,
                (t * 67.0).cos() * 0.20,
                (t * 41.0).sin() * 0.16,
            ) * amp
                * scope_scale
                + s.kick_dir * (amp * 0.28 * scope_scale);
            let jitter_rot = Quat::from_euler(
                EulerRot::YXZ,
                (t * 47.0).cos() * 0.012 * amp * scope_scale,
                (t * 61.0).sin() * 0.018 * amp * scope_scale,
                (t * 59.0).cos() * 0.014 * amp * scope_scale,
            );
            cam_tf.translation += jitter_pos;
            cam_tf.rotation *= jitter_rot;
            s.trauma = (s.trauma - dt * 2.25).max(0.0);
        } else {
            s.kick_dir = Vec3::ZERO;
        }
    }
}

/// Пересчитывает точку сведения ствола (`aim.gun_point`) ПОСЛЕ `aim_turret_and_gun`,
/// используя актуальные локальные трансформации башни и орудия текущего кадра.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update_gun_aim_point(
    tanks: Query<(Entity, &GlobalTransform), (With<TankRoot>, With<PlayerTank>)>,
    turrets: Query<(&Transform, &TurretPivot), Without<GunPivot>>,
    guns: Query<(&Transform, &GunPivot), Without<TurretPivot>>,
    muzzles: Query<(&Transform, &GlobalTransform, &MuzzlePoint), (Without<TurretPivot>, Without<GunPivot>)>,
    world: Option<Res<VoxelWorld>>,
    plates: Query<(&GlobalTransform, &ArmorPlate, &TargetInfo, Option<&ArmorOwner>)>,
    mut aim: ResMut<AimTarget>,
) {
    let Ok((player_ent, tank_gt)) = tanks.single() else {
        aim.gun_point = aim.point;
        return;
    };
    let Some((m_tf, m_gt, _)) = muzzles.iter().find(|(_, _, m)| m.tank == player_ent) else {
        aim.gun_point = aim.point;
        return;
    };

    let current_muzzle = if let (Some((t_tf, _)), Some((g_tf, _))) = (
        turrets.iter().find(|(_, t)| t.tank == player_ent),
        guns.iter().find(|(_, g)| g.tank == player_ent),
    ) {
        compute_gun_and_muzzle_world(tank_gt, t_tf, g_tf, m_tf).1
    } else {
        *m_gt
    };

    let m_pos = current_muzzle.translation();
    let m_dir = current_muzzle.forward().as_vec3();
    let (gpt, _, _, _) = raycast_aim(
        m_pos,
        m_dir,
        0.2,
        Some(player_ent),
        world.as_deref(),
        &plates,
    );
    aim.muzzle_origin = m_pos;
    aim.muzzle_dir = m_dir;
    aim.gun_point = gpt;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tank::spec::load_tank_spec;
    use crate::tank::turret::aim_turret_and_gun;
    use bevy::input::InputPlugin;
    use bevy::state::app::StatesPlugin;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    #[test]
    fn horizon_camera_and_gun_coincide_in_third_person_and_sniper() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin, TransformPlugin, StatesPlugin))
            .init_resource::<AimTarget>()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                1.0 / 60.0,
            )))
            .add_systems(
                Update,
                (update_tank_camera, aim_turret_and_gun, update_gun_aim_point).chain(),
            );

        let spec = load_tank_spec("tiger_2").expect("tiger_2 spec");
        let root_pos = Vec3::new(0.0, 3.0, 140.0);
        let root_ent = app.world_mut().spawn_empty().id();
        let turret_offset = Vec3::from_array(spec.profile.turret_pivot);
        let gun_offset = Vec3::from_array(spec.profile.gun_pivot);
        let muzzle_offset = Vec3::new(0.0, 0.0, -(spec.profile.barrel_length + 0.15));

        app.world_mut().entity_mut(root_ent).insert((
            Transform::from_translation(root_pos),
            GlobalTransform::from_translation(root_pos),
            TankRoot {
                entity: root_ent,
                spec: spec.clone(),
                spawn_pos: root_pos,
                spawn_yaw: 0.0,
            },
            PlayerTank,
            AimCommand {
                world_point: Vec3::new(0.0, 5.65, -660.0),
            },
        ));
        let turret_ent = app
            .world_mut()
            .spawn((
                TurretPivot { tank: root_ent },
                Transform::from_translation(turret_offset),
            ))
            .id();
        let gun_ent = app
            .world_mut()
            .spawn((
                GunPivot { tank: root_ent },
                Transform::from_translation(gun_offset),
            ))
            .id();
        let muzzle_ent = app
            .world_mut()
            .spawn((
                MuzzlePoint { tank: root_ent },
                Transform::from_translation(muzzle_offset),
            ))
            .id();
        app.world_mut().entity_mut(gun_ent).add_child(muzzle_ent);
        app.world_mut().entity_mut(turret_ent).add_child(gun_ent);
        app.world_mut().entity_mut(root_ent).add_child(turret_ent);

        let cam_ent = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                Projection::Perspective(PerspectiveProjection {
                    fov: 60f32.to_radians(),
                    ..default()
                }),
                Transform::default(),
                TankCamera {
                    yaw: 0.0,
                    pitch: 0.0, // Смотрим ровно в горизонт.
                    distance: 11.5,
                    sniper: false,
                },
            ))
            .id();

        app.finish();
        app.cleanup();

        for mode_sniper in [false, true] {
            app.world_mut().get_mut::<TankCamera>(cam_ent).unwrap().sniper = mode_sniper;
            for _ in 0..30 {
                app.update();
            }
            let cam_tf = *app.world().get::<Transform>(cam_ent).unwrap();
            let aim = app.world().resource::<AimTarget>();
            let inv_cam = cam_tf.to_matrix().inverse();

            let p_view = inv_cam.transform_point3(aim.point);
            let g_view = inv_cam.transform_point3(aim.gun_point);
            assert!(p_view.z < -10.0 && g_view.z < -10.0);

            let ndc_p = Vec2::new(p_view.x / -p_view.z, p_view.y / -p_view.z);
            let ndc_g = Vec2::new(g_view.x / -g_view.z, g_view.y / -g_view.z);
            assert!(
                ndc_p.length() < 1e-4,
                "центр прицела (sniper={mode_sniper}) должен лежать на оси камеры: {ndc_p:?}"
            );
            assert!(
                (ndc_g - ndc_p).length() < 0.002,
                "орудие (sniper={mode_sniper}) должно совпадать с центром прицела на горизонте: ndc_p={ndc_p:?}, ndc_g={ndc_g:?}"
            );
            assert!(aim.is_converged());
        }
    }
}
