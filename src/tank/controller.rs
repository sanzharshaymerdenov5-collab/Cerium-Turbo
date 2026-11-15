//! Управление танком: пружинная подвеска, гусеничная тяга, стрельба, отдача, возврат на колёса.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::armor::TankCombat;
use super::spawn::{GunPivot, MuzzlePoint, PlayerTank, Suspension, TankRoot, TankState, TurretPivot};
use super::turret::compute_gun_and_muzzle_world;
use crate::ballistics::{HitLog, spawn_projectile};
use crate::core::Team;
use crate::voxel::{Material, VoxelAssets, VoxelWorld};

/// Событие выстрела танка (для эффектов дульной вспышки, отката ствола, тряски камеры и звука).
#[derive(Message, Debug, Clone)]
pub struct TankFiredMessage {
    pub tank: Entity,
    pub team: Option<Team>,
    pub muzzle_pos: Vec3,
    pub muzzle_dir: Vec3,
    pub gun_pivot: Vec3,
    pub caliber_mm: f32,
    pub is_player: bool,
    pub has_muzzle_brake: bool,
}

/// Чистая формула вертикальной силы пружинно-демпферной стойки:
/// `compression > 0` — сжатие (м), `rel_vel_up` — скорость точки крепления вдоль нормали вверх (м/с).
/// Стойка только толкает вверх (сила ≥ 0) и ограничена `max_force`.
pub fn compute_spring_force(
    compression: f32,
    rel_vel_up: f32,
    spring_k: f32,
    damper_c: f32,
    max_force: f32,
) -> f32 {
    if compression <= 0.0 || !compression.is_finite() || !rel_vel_up.is_finite() {
        return 0.0;
    }
    (compression * spring_k - rel_vel_up * damper_c).clamp(0.0, max_force)
}

/// Читает ввод игрока в `Update` (чтобы `just_pressed` не терялся между кадрами физики).
pub fn read_player_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut q: Query<(&mut TankState, &TankCombat), With<PlayerTank>>,
) {
    let Ok((mut st, combat)) = q.single_mut() else {
        return;
    };
    if !combat.alive {
        st.throttle = 0.0;
        st.steer = 0.0;
        st.fire_queued = false;
        st.reset_queued = false;
        return;
    }
    let mut throttle = 0.0;
    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        throttle += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) {
        throttle -= 1.0;
    }
    let mut steer = 0.0;
    if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) {
        steer += 1.0;
    }
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        steer -= 1.0;
    }
    st.throttle = throttle;
    st.steer = steer;
    if keys.just_pressed(KeyCode::Space) || mouse.just_pressed(MouseButton::Left) {
        st.fire_queued = true;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        st.reset_queued = true;
    }
}

/// Физический шаг подвески и гусеничного привода (`FixedUpdate`).
pub fn tank_suspension_and_drive(
    time: Res<Time>,
    world: Option<Res<VoxelWorld>>,
    mut q: Query<(
        &mut Transform,
        &TankRoot,
        &Suspension,
        &mut TankState,
        Option<&TankCombat>,
        Forces,
    )>,
) {
    let Some(world) = world else { return };
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    for (mut tf, root, susp, mut st, combat, mut forces) in &mut q {
        let alive = combat.is_none_or(|c| c.alive);
        let immobilized = combat.is_some_and(|c| c.immobilized);
        let speed_mult = combat.map_or(1.0, |c| c.speed_mult);

        if st.reset_queued && alive {
            st.reset_queued = false;
            let ground_y = world
                .grid
                .surface_height_at(tf.translation.x, tf.translation.z)
                .unwrap_or(root.spawn_pos.y);
            tf.translation = Vec3::new(tf.translation.x, ground_y + 0.5, tf.translation.z);
            let (yaw, _, _) = tf.rotation.to_euler(EulerRot::YXZ);
            tf.rotation = Quat::from_rotation_y(yaw);
            *forces.linear_velocity_mut() = Vec3::ZERO;
            *forces.angular_velocity_mut() = Vec3::ZERO;
            continue;
        }
        st.reset_queued = false;

        let rot = tf.rotation;
        let pos = tf.translation;
        let up = rot * Vec3::Y;
        let forward = rot * Vec3::NEG_Z;
        let right = rot * Vec3::X;
        let lin_vel = forces.linear_velocity();
        let ang_vel = forces.angular_velocity();

        st.speed_kmh = lin_vel.dot(forward) * 3.6;

        // Если танк сильно накренился, мягкий стабилизирующий момент возвращает его.
        if up.y < 0.85 {
            let corr = up.cross(Vec3::Y) * (root.spec.mass_kg * 9.81 * 1.4);
            forces.apply_torque(corr);
        }

        let mut grounded = 0u8;
        let mut grounded_points = [Vec3::ZERO; 16];
        let mut norm_sum = 0.0f32;
        let mut traction_sum = 0.0f32;

        for w in &susp.wheels {
            let wp = pos + rot * w.local_pos;
            let Some(hit) = world.grid.raycast(wp, Vec3::NEG_Y, susp.max_drop) else {
                continue;
            };
            let dist = hit.distance;
            if dist > susp.rest_length {
                continue;
            }
            let compression = susp.rest_length - dist;
            let r = wp - pos;
            let vel_at_wheel = lin_vel + ang_vel.cross(r);
            let f_norm = compute_spring_force(
                compression,
                vel_at_wheel.y,
                susp.spring_k,
                susp.damper_c,
                susp.max_force,
            );
            forces.apply_force_at_point(Vec3::Y * f_norm, wp);

            if (grounded as usize) < grounded_points.len() {
                grounded_points[grounded as usize] = wp;
            }
            grounded += 1;
            norm_sum += f_norm;
            traction_sum += if hit.material == Material::Ice { 0.25 } else { 1.0 };
        }

        st.grounded_wheels = grounded;
        if grounded == 0 {
            st.surface_traction = 0.0;
            continue;
        }

        let traction = (traction_sum / grounded as f32).clamp(0.15, 1.0);
        st.surface_traction = traction;
        let g_ratio = (grounded as f32 / susp.wheels.len().max(1) as f32).clamp(0.0, 1.0);

        let eff_throttle = if alive && !immobilized {
            st.throttle.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let eff_steer = if alive && !immobilized {
            st.steer.clamp(-1.0, 1.0)
        } else {
            0.0
        };

        let max_fwd = (root.spec.max_speed_kmh / 3.6) * speed_mult;
        let max_rev = (root.spec.reverse_speed_kmh / 3.6) * speed_mult;
        let raw_target_fwd = if eff_throttle >= 0.0 {
            eff_throttle * max_fwd
        } else {
            eff_throttle * max_rev
        };
        // При одновременном газе и повороте немного сбрасываем целевую скорость на внутреннюю гусеницу.
        let target_fwd = raw_target_fwd * (1.0 - 0.14 * eff_steer.abs());

        let com_fwd_v = lin_vel.dot(forward);
        let com_lat_v = lin_vel.dot(right);
        let max_total_drive =
            root.spec.max_tractive_force_kn * 1000.0 * traction * g_ratio * speed_mult.max(0.2);

        let v_err = target_fwd - com_fwd_v;
        let long_gain = if eff_throttle.abs() > 0.01 { 2.8 } else { 1.2 };
        let f_long_total =
            (v_err * root.spec.mass_kg * long_gain * traction).clamp(-max_total_drive, max_total_drive);

        // Гасим боковой снос в центре масс (НЕ используем `vel_at_wheel`, чтобы не тормозить поворот вокруг Y!).
        let max_lat_total = norm_sum.max(root.spec.mass_kg * 5.0) * 1.15 * traction;
        let f_lat_total =
            (-com_lat_v * root.spec.mass_kg * 2.8 * traction).clamp(-max_lat_total, max_lat_total);

        let inv_g = 1.0 / (grounded.min(grounded_points.len() as u8) as f32);
        let wheel_horiz = (forward * f_long_total + right * f_lat_total) * inv_g;
        for &wp in grounded_points.iter().take(grounded as usize) {
            forces.apply_force_at_point(wheel_horiz, wp);
        }

        // Явный сервомомент гусеничного разворота вокруг локальной оси +Y корпуса.
        let track_span = (root.spec.profile.hull_width - root.spec.profile.track_width).max(1.5);
        let steer_dir = if eff_throttle < -0.1 { -1.0 } else { 1.0 };
        let target_yaw = eff_steer
            * steer_dir
            * root.spec.max_yaw_rate_dps.to_radians()
            * speed_mult.max(0.5);
        let cur_yaw = ang_vel.dot(up);
        let yaw_inertia = root.spec.mass_kg
            * (root.spec.profile.hull_width.powi(2) + root.spec.profile.hull_length.powi(2))
            / 12.0;
        let yaw_gain = if eff_steer.abs() > 0.01 { 8.0 } else { 5.5 };
        let max_yaw_torque = (root.spec.max_tractive_force_kn * 1000.0)
            * (track_span * 0.5)
            * 1.45
            * traction
            * g_ratio;
        let yaw_torque =
            ((target_yaw - cur_yaw) * yaw_inertia * yaw_gain).clamp(-max_yaw_torque, max_yaw_torque);
        forces.apply_torque(up * yaw_torque);
    }
}

/// Перезарядка, выстрел и импульс отдачи.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn tank_fire_system(
    mut commands: Commands,
    time: Res<Time>,
    assets: Option<Res<VoxelAssets>>,
    mut log: ResMut<HitLog>,
    mut tanks: Query<(
        &GlobalTransform,
        &TankRoot,
        &mut TankState,
        Option<&TankCombat>,
        Option<&PlayerTank>,
        Forces,
    )>,
    turrets: Query<(&Transform, &TurretPivot)>,
    guns: Query<(&Transform, &GlobalTransform, &GunPivot)>,
    muzzles: Query<(&Transform, &GlobalTransform, &MuzzlePoint)>,
    mut fired_w: MessageWriter<TankFiredMessage>,
) {
    let Some(assets) = assets else { return };
    let dt = time.delta_secs();

    for (root_gt, root, mut st, combat, player_opt, mut forces) in &mut tanks {
        let alive = combat.is_none_or(|c| c.alive);
        let jammed = combat.is_some_and(|c| c.gun_jammed);
        let reload_mult = combat.map_or(1.0, |c| c.reload_mult.max(0.5));
        let team = combat.map(|c| c.team);

        st.reload_timer = (st.reload_timer - dt / reload_mult).max(0.0);
        if !st.fire_queued {
            continue;
        }
        st.fire_queued = false;
        if !alive || jammed || st.reload_timer > 0.0 {
            continue;
        }
        st.reload_timer = root.spec.reload_s * reload_mult;
        if player_opt.is_some() {
            log.shots += 1;
        }

        let Some((m_tf, m_gt, _)) = muzzles.iter().find(|(_, _, m)| m.tank == root.entity) else {
            continue;
        };
        let (recoil_pt, current_muzzle) = if let (Some((t_tf, _)), Some((g_tf, _, _))) = (
            turrets.iter().find(|(_, t)| t.tank == root.entity),
            guns.iter().find(|(_, _, g)| g.tank == root.entity),
        ) {
            compute_gun_and_muzzle_world(root_gt, t_tf, g_tf, m_tf)
        } else {
            let fallback_pivot = guns
                .iter()
                .find(|(_, _, g)| g.tank == root.entity)
                .map(|(_, gt, _)| gt.translation())
                .unwrap_or_else(|| m_gt.translation());
            (fallback_pivot, *m_gt)
        };
        let origin = current_muzzle.translation();
        let dir = current_muzzle.forward().as_vec3();

        spawn_projectile(
            &mut commands,
            &assets,
            origin,
            dir,
            root.spec.shell,
            Some(root.entity),
            team,
        );

        // Импульс отдачи в точке цапф орудия.
        forces.apply_linear_impulse_at_point(-dir * root.spec.recoil_impulse_ns, recoil_pt);

        let has_muzzle_brake = root.spec.profile.has_muzzle_brake();
        fired_w.write(TankFiredMessage {
            tank: root.entity,
            team,
            muzzle_pos: origin,
            muzzle_dir: dir,
            gun_pivot: recoil_pt,
            caliber_mm: root.spec.shell.caliber_mm,
            is_player: player_opt.is_some(),
            has_muzzle_brake,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::tank::spawn::spawn_tank;
    use crate::tank::spec::load_tank_spec;
    use crate::voxel::VoxelGrid;
    use bevy::state::app::StatesPlugin;
    use bevy::time::TimeUpdateStrategy;

    #[test]
    fn spring_force_is_finite_non_negative_and_monotonic() {
        assert_eq!(compute_spring_force(0.0, 0.0, 200_000.0, 30_000.0, 500_000.0), 0.0);
        assert_eq!(compute_spring_force(-0.2, -5.0, 200_000.0, 30_000.0, 500_000.0), 0.0);
        assert_eq!(
            compute_spring_force(f32::NAN, 0.0, 200_000.0, 30_000.0, 500_000.0),
            0.0
        );

        let f1 = compute_spring_force(0.05, 0.0, 200_000.0, 30_000.0, 500_000.0);
        let f2 = compute_spring_force(0.15, 0.0, 200_000.0, 30_000.0, 500_000.0);
        assert!(f2 > f1 && f1 > 0.0);

        let rebound = compute_spring_force(0.05, 20.0, 200_000.0, 30_000.0, 500_000.0);
        assert_eq!(rebound, 0.0);
    }

    fn run_drive_sim(throttle: f32, steer: f32, steps: usize) -> (Vec3, f32) {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            StatesPlugin,
            PhysicsPlugins::default(),
        ))
        .init_state::<crate::core::GameState>()
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_resource::<VoxelAssets>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )))
        .add_systems(FixedUpdate, tank_suspension_and_drive);

        // Ровная воксельная площадка 64x8x64 с шагом 1.0 м, пол до y=2 (верх y=2.0 м).
        let mut grid = VoxelGrid::new(UVec3::new(64, 8, 64), 1.0, Vec3::new(-32.0, 0.0, -32.0));
        grid.fill_box(IVec3::new(0, 0, 0), IVec3::new(63, 1, 63), Material::Grass);
        let world = VoxelWorld::new(grid);
        app.insert_resource(world);
        app.world_mut()
            .resource_mut::<NextState<crate::core::GameState>>()
            .set(crate::core::GameState::InBattle);

        app.add_systems(
            Startup,
            |mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, assets: Res<VoxelAssets>| {
                let spec = load_tank_spec("t34_85").unwrap();
                spawn_tank(
                    &mut commands,
                    &mut meshes,
                    &assets,
                    spec,
                    Vec3::new(0.0, 2.2, 0.0),
                    0.0,
                    true,
                );
            },
        );
        app.finish();
        app.cleanup();

        for _ in 0..steps {
            {
                let mut q = app.world_mut().query::<&mut TankState>();
                for mut st in q.iter_mut(app.world_mut()) {
                    st.throttle = throttle;
                    st.steer = steer;
                }
            }
            app.update();
        }

        let mut q = app.world_mut().query::<(&Transform, &TankRoot)>();
        let (tf, _) = q.single(app.world()).unwrap();
        let (yaw, _, _) = tf.rotation.to_euler(EulerRot::YXZ);
        (tf.translation, yaw)
    }

    #[test]
    fn tank_turns_while_driving_forward_w_plus_a_and_w_plus_d() {
        // W+A: движение вперёд (-Z) и поворот влево (+yaw).
        let (pos_wa, yaw_wa) = run_drive_sim(1.0, 1.0, 90);
        assert!(pos_wa.z < -3.0, "W+A должен ехать вперёд, pos={pos_wa:?}");
        assert!(
            yaw_wa.to_degrees() > 20.0,
            "W+A должен поворачивать влево (>20°), yaw={}°",
            yaw_wa.to_degrees()
        );

        // W+D: движение вперёд (-Z) и поворот вправо (-yaw).
        let (pos_wd, yaw_wd) = run_drive_sim(1.0, -1.0, 90);
        assert!(pos_wd.z < -3.0, "W+D должен ехать вперёд, pos={pos_wd:?}");
        assert!(
            yaw_wd.to_degrees() < -20.0,
            "W+D должен поворачивать вправо (<-20°), yaw={}°",
            yaw_wd.to_degrees()
        );

        // Чистый разворот на месте (A).
        let (_, yaw_a) = run_drive_sim(0.0, 1.0, 60);
        assert!(
            yaw_a.to_degrees() > 15.0,
            "A должен поворачивать на месте (>15°), yaw={}°",
            yaw_a.to_degrees()
        );
    }
}
